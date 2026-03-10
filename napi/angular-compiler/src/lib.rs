//! NAPI bindings for the Angular compiler.
//!
//! This crate provides Node.js bindings for the Oxc Angular compiler,
//! enabling fast template compilation and HMR support in build tools.

#![allow(clippy::needless_pass_by_value)]

#[cfg(all(
    feature = "allocator",
    not(any(target_arch = "arm", target_os = "freebsd", target_family = "wasm"))
))]
#[global_allocator]
static ALLOC: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

use std::collections::HashMap;

use napi::{Task, bindgen_prelude::AsyncTask};
use napi_derive::napi;

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    AngularVersion as RustAngularVersion, ChangeDetectionStrategy as RustChangeDetectionStrategy,
    HostMetadataInput as RustHostMetadataInput, TransformOptions as RustTransformOptions,
    ViewEncapsulation as RustViewEncapsulation,
    build_ctor_params_metadata as core_build_ctor_params_metadata,
    build_decorator_metadata_array as core_build_decorator_metadata_array,
    build_prop_decorators_metadata as core_build_prop_decorators_metadata,
    compile_template_for_hmr, compile_template_to_js_with_options,
    encapsulate_style as rust_encapsulate_style, generate_hmr_update_module_from_js,
    generate_style_update_module, shim_css_text,
};
use oxc_napi::OxcError;

/// Angular version for version-conditional behavior.
#[derive(Default, Clone)]
#[napi(object)]
pub struct AngularVersion {
    /// Major version number (e.g., 19 for Angular 19.0.0).
    pub major: u32,
    /// Minor version number (e.g., 0 for Angular 19.0.0).
    pub minor: u32,
    /// Patch version number (e.g., 0 for Angular 19.0.0).
    pub patch: u32,
}

impl From<AngularVersion> for RustAngularVersion {
    fn from(v: AngularVersion) -> Self {
        RustAngularVersion::new(v.major, v.minor, v.patch)
    }
}

/// Host bindings metadata for a component.
///
/// Contains property bindings, attribute bindings, and event listeners
/// extracted from the `host` property of a `@Component` decorator.
#[derive(Default, Clone)]
#[napi(object)]
pub struct HostMetadataInput {
    /// Host property bindings: `[[key, value], ...]`.
    /// Example: `[["[class.active]", "isActive"], ["[disabled]", "isDisabled"]]`
    pub properties: Option<Vec<Vec<String>>>,

    /// Host attribute bindings: `[[key, value], ...]`.
    /// Example: `[["role", "button"], ["aria-label", "Submit"]]`
    pub attributes: Option<Vec<Vec<String>>>,

    /// Host event listeners: `[[key, value], ...]`.
    /// Example: `[["(click)", "onClick()"], ["(keydown.enter)", "onEnter()"]]`
    pub listeners: Option<Vec<Vec<String>>>,

    /// Special attribute for static class binding.
    pub class_attr: Option<String>,

    /// Special attribute for static style binding.
    pub style_attr: Option<String>,
}

impl From<HostMetadataInput> for RustHostMetadataInput {
    fn from(h: HostMetadataInput) -> Self {
        Self {
            properties: h
                .properties
                .unwrap_or_default()
                .into_iter()
                .filter_map(|pair| {
                    if pair.len() >= 2 { Some((pair[0].clone(), pair[1].clone())) } else { None }
                })
                .collect(),
            attributes: h
                .attributes
                .unwrap_or_default()
                .into_iter()
                .filter_map(|pair| {
                    if pair.len() >= 2 { Some((pair[0].clone(), pair[1].clone())) } else { None }
                })
                .collect(),
            listeners: h
                .listeners
                .unwrap_or_default()
                .into_iter()
                .filter_map(|pair| {
                    if pair.len() >= 2 { Some((pair[0].clone(), pair[1].clone())) } else { None }
                })
                .collect(),
            class_attr: h.class_attr,
            style_attr: h.style_attr,
        }
    }
}

/// Options for transforming an Angular component.
#[derive(Default)]
#[napi(object)]
pub struct TransformOptions {
    /// Generate source maps.
    pub sourcemap: Option<bool>,

    /// Enable JIT (Just-In-Time) compilation mode.
    pub jit: Option<bool>,

    /// Enable HMR (Hot Module Replacement) support.
    pub hmr: Option<bool>,

    /// Enable advanced optimizations.
    pub advanced_optimizations: Option<bool>,

    /// i18n message ID strategy.
    ///
    /// When true (default), uses external message IDs (MSG_EXTERNAL_abc123$$SUFFIX).
    /// When false, uses file-based naming (MSG_SUFFIX_0).
    pub i18n_use_external_ids: Option<bool>,

    /// Angular core version for version-conditional behavior.
    ///
    /// When set, used to determine defaults like:
    /// - `standalone`: defaults to `false` for v18 and earlier, `true` for v19+
    ///
    /// When not set, assumes latest Angular version (v19+ behavior).
    pub angular_version: Option<AngularVersion>,

    // Component metadata fields for full compilation testing
    // These mirror Angular's @Component decorator options.
    /// The CSS selector that identifies this component in a template.
    pub selector: Option<String>,

    /// Whether this component is standalone.
    /// When not set, defaults based on Angular version (true for v19+, false for v18-).
    pub standalone: Option<bool>,

    /// View encapsulation mode: "Emulated" | "None" | "ShadowDom".
    /// Defaults to "Emulated" if not specified.
    pub encapsulation: Option<String>,

    /// Change detection strategy: "Default" | "OnPush".
    /// Defaults to "Default" if not specified.
    pub change_detection: Option<String>,

    /// Whether to preserve whitespace in templates.
    /// Defaults to false if not specified.
    pub preserve_whitespaces: Option<bool>,

    /// Host bindings metadata for the component.
    /// Contains property bindings, attribute bindings, and event listeners.
    pub host: Option<HostMetadataInput>,

    /// Enable cross-file import elision analysis.
    ///
    /// When true, resolves imports to source files to check if exports are type-only.
    /// This improves import elision accuracy for compare tests.
    ///
    /// **Note**: This is intended for compare tests only. In production, bundlers
    /// handle import elision during tree-shaking.
    pub cross_file_elision: Option<bool>,

    /// Base directory for module resolution.
    ///
    /// Used when `cross_file_elision` is enabled to resolve relative imports.
    pub base_dir: Option<String>,

    /// Path to tsconfig.json for path aliases.
    ///
    /// Used when `cross_file_elision` is enabled to resolve path aliases.
    pub tsconfig_path: Option<String>,

    /// Emit setClassMetadata() calls for TestBed support.
    ///
    /// When true, generates `ɵɵsetClassMetadata()` calls wrapped in a dev-mode guard.
    /// This preserves original decorator information for TestBed's recompilation APIs.
    ///
    /// Default: false (metadata is dev-only and usually stripped in production)
    pub emit_class_metadata: Option<bool>,

    /// Resolved import paths for host directives and other imports.
    ///
    /// Maps local identifier name (e.g., "AriaDisableDirective") to the resolved
    /// module path (e.g., "../a11y/aria-disable.directive").
    ///
    /// This is used to override barrel export paths with actual file paths.
    /// The build tool should resolve imports using TypeScript's module resolution
    /// and provide the actual file paths here.
    #[napi(ts_type = "Map<string, string>")]
    pub resolved_imports: Option<HashMap<String, String>>,
}

impl From<TransformOptions> for RustTransformOptions {
    fn from(options: TransformOptions) -> Self {
        Self {
            sourcemap: options.sourcemap.unwrap_or(false),
            jit: options.jit.unwrap_or(false),
            hmr: options.hmr.unwrap_or(false),
            advanced_optimizations: options.advanced_optimizations.unwrap_or(false),
            i18n_use_external_ids: options.i18n_use_external_ids.unwrap_or(true),
            angular_version: options.angular_version.map(Into::into),
            // Component metadata overrides
            selector: options.selector,
            standalone: options.standalone,
            encapsulation: options.encapsulation.and_then(|s| parse_view_encapsulation(&s)),
            change_detection: options
                .change_detection
                .and_then(|s| parse_change_detection_strategy(&s)),
            preserve_whitespaces: options.preserve_whitespaces,
            host: options.host.map(Into::into),
            // Cross-file elision options (feature-gated in Rust, always available via NAPI)
            #[cfg(feature = "cross_file_elision")]
            cross_file_elision: options.cross_file_elision.unwrap_or(false),
            #[cfg(feature = "cross_file_elision")]
            base_dir: options.base_dir.map(std::path::PathBuf::from),
            #[cfg(feature = "cross_file_elision")]
            tsconfig_path: options.tsconfig_path.map(std::path::PathBuf::from),
            // Resolved imports for host directives
            resolved_imports: options.resolved_imports,
            // Class metadata for TestBed support
            emit_class_metadata: options.emit_class_metadata.unwrap_or(false),
        }
    }
}

/// Parse a ViewEncapsulation string to the Rust enum.
///
/// Valid values: "Emulated", "None", "ShadowDom"
fn parse_view_encapsulation(s: &str) -> Option<RustViewEncapsulation> {
    match s {
        "Emulated" => Some(RustViewEncapsulation::Emulated),
        "None" => Some(RustViewEncapsulation::None),
        "ShadowDom" => Some(RustViewEncapsulation::ShadowDom),
        _ => None,
    }
}

/// Parse a ChangeDetectionStrategy string to the Rust enum.
///
/// Valid values: "Default", "OnPush"
fn parse_change_detection_strategy(s: &str) -> Option<RustChangeDetectionStrategy> {
    match s {
        "Default" => Some(RustChangeDetectionStrategy::Default),
        "OnPush" => Some(RustChangeDetectionStrategy::OnPush),
        _ => None,
    }
}

/// Result of compiling an Angular template.
#[derive(Default)]
#[napi(object)]
pub struct TemplateCompileResult {
    /// The compiled template function as JavaScript code.
    pub code: String,

    /// Source map (if sourcemap option was enabled).
    pub map: Option<String>,

    /// Compilation errors.
    pub errors: Vec<OxcError>,
}

/// A `.d.ts` type declaration for an Angular class.
///
/// Contains the class name and the static member declarations
/// that should be injected into the corresponding `.d.ts` class body.
#[derive(Default)]
#[napi(object)]
pub struct DtsDeclaration {
    /// The name of the class.
    pub class_name: String,
    /// The static member declarations to add to the class body in `.d.ts`.
    /// Newline-separated `static` property declarations.
    pub members: String,
}

/// Result of transforming an Angular file.
#[derive(Default)]
#[napi(object)]
pub struct TransformResult {
    /// The transformed code.
    pub code: String,

    /// Source map (if sourcemap option was enabled).
    pub map: Option<String>,

    /// Files this file depends on (for watch mode).
    pub dependencies: Vec<String>,

    /// Template updates for HMR (component_id → compiled_template).
    #[napi(ts_type = "Map<string, string>")]
    pub template_updates: HashMap<String, String>,

    /// Style updates for HMR (component_id → styles).
    #[napi(ts_type = "Map<string, string[]>")]
    pub style_updates: HashMap<String, Vec<String>>,

    /// Compilation errors.
    pub errors: Vec<OxcError>,

    /// Compilation warnings.
    pub warnings: Vec<OxcError>,

    /// `.d.ts` type declarations for Angular classes.
    ///
    /// Each entry contains the class name and the static member declarations
    /// that should be injected into the corresponding `.d.ts` class body.
    /// This enables library builds to include proper Ivy type declarations
    /// for template type-checking by consumers.
    ///
    /// The declarations use `i0` as the namespace alias for `@angular/core`.
    /// Consumers must ensure their `.d.ts` files include:
    /// `import * as i0 from "@angular/core";`
    pub dts_declarations: Vec<DtsDeclaration>,
}

/// Compile an Angular template to JavaScript.
///
/// This compiles a template string to a template function that can be
/// used in Angular's component definition.
///
/// # Arguments
///
/// * `template` - The template HTML string
/// * `component_name` - The name of the component class
/// * `file_path` - The path to the component file (for error messages)
///
/// # Returns
///
/// A `TemplateCompileResult` containing the compiled code or errors.
pub fn compile_template_sync(
    template: String,
    component_name: String,
    file_path: String,
    options: Option<TransformOptions>,
) -> TemplateCompileResult {
    let allocator = Allocator::default();
    let opts: RustTransformOptions = options.unwrap_or_default().into();

    match compile_template_to_js_with_options(
        &allocator,
        &template,
        &component_name,
        &file_path,
        &opts,
    ) {
        Ok(output) => {
            // Convert source map to JSON string if present
            let map = output.map.map(|m| m.to_json_string());
            TemplateCompileResult { code: output.code, map, errors: vec![] }
        }
        Err(diagnostics) => TemplateCompileResult {
            code: String::new(),
            map: None,
            errors: OxcError::from_diagnostics(&file_path, &template, diagnostics),
        },
    }
}

/// Async task for template compilation.
pub struct CompileTemplateTask {
    template: String,
    component_name: String,
    file_path: String,
    options: TransformOptions,
}

#[napi]
impl Task for CompileTemplateTask {
    type JsValue = TemplateCompileResult;
    type Output = TemplateCompileResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(compile_template_sync(
            self.template.clone(),
            self.component_name.clone(),
            self.file_path.clone(),
            Some(std::mem::take(&mut self.options)),
        ))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Compile an Angular template to JavaScript (async).
///
/// This is the async version of `compileTemplateSync`. Use this when
/// compiling templates in a non-blocking context.
#[napi]
pub fn compile_template(
    template: String,
    component_name: String,
    file_path: String,
    options: Option<TransformOptions>,
) -> AsyncTask<CompileTemplateTask> {
    AsyncTask::new(CompileTemplateTask {
        template,
        component_name,
        file_path,
        options: options.unwrap_or_default(),
    })
}

/// Generate an HMR update module for a component.
///
/// This generates a JavaScript module that can be dynamically imported
/// during HMR to update a component's template and styles.
///
/// # Arguments
///
/// * `component_id` - The component ID (path@ClassName)
/// * `template_js` - The compiled template function as JavaScript
/// * `styles` - Optional array of CSS styles
///
/// # Returns
///
/// JavaScript code for the HMR update module.
#[napi]
pub fn generate_hmr_module(
    component_id: String,
    template_js: String,
    styles: Option<Vec<String>>,
    declarations_js: Option<String>,
    consts_js: Option<String>,
) -> String {
    generate_hmr_update_module_from_js(
        &component_id,
        &template_js,
        styles.as_deref(),
        declarations_js.as_deref(),
        consts_js.as_deref(),
    )
}

/// Generate a style update module for HMR.
///
/// This generates a JavaScript module that exports updated styles
/// for a component.
///
/// # Arguments
///
/// * `component_id` - The component ID (path@ClassName)
/// * `styles` - Array of CSS styles
///
/// # Returns
///
/// JavaScript code for the style update module.
#[napi]
pub fn generate_style_module(component_id: String, styles: Vec<String>) -> String {
    generate_style_update_module(&component_id, &styles)
}

/// Transform a template and generate HMR module in one step.
///
/// This is a convenience function that combines template compilation
/// and HMR module generation.
///
/// # Arguments
///
/// * `template` - The template HTML string
/// * `component_name` - The name of the component class
/// * `file_path` - The path to the component file
/// * `styles` - Optional array of CSS styles
///
/// # Returns
///
/// A tuple of (hmr_module_code, component_id) or errors.
#[napi]
pub fn compile_for_hmr_sync(
    template: String,
    component_name: String,
    file_path: String,
    styles: Option<Vec<String>>,
    options: Option<TransformOptions>,
) -> HmrCompileResult {
    let allocator = Allocator::default();
    let opts: RustTransformOptions = options.unwrap_or_default().into();

    // Generate component ID
    let component_id = format!("{file_path}@{component_name}");

    // Compile template for HMR (returns template function, declarations, and template styles)
    match compile_template_for_hmr(&allocator, &template, &component_name, &file_path, &opts) {
        Ok(output) => {
            let template_js = output.template_js;
            let declarations_js = if output.declarations_js.is_empty() {
                None
            } else {
                Some(output.declarations_js.as_str())
            };

            // Merge external styles with styles extracted from template <style> tags
            let mut all_styles: Vec<String> = styles.unwrap_or_default();
            all_styles.extend(output.styles);

            // Apply style encapsulation for ViewEncapsulation.Emulated
            // Angular uses %COMP% as a placeholder that the runtime replaces with the component ID
            let encapsulated_styles: Option<Vec<String>> = if all_styles.is_empty() {
                None
            } else {
                Some(
                    all_styles
                        .iter()
                        .map(|style| shim_css_text(style, "_ngcontent-%COMP%", "_nghost-%COMP%"))
                        .collect(),
                )
            };

            // Generate HMR module with declarations, encapsulated styles, and consts
            let hmr_module = generate_hmr_update_module_from_js(
                &component_id,
                &template_js,
                encapsulated_styles.as_deref(),
                declarations_js,
                output.consts_js.as_deref(),
            );

            HmrCompileResult { hmr_module, component_id, template_js, errors: vec![] }
        }
        Err(diagnostics) => HmrCompileResult {
            hmr_module: String::new(),
            component_id,
            template_js: String::new(),
            errors: OxcError::from_diagnostics(&file_path, &template, diagnostics),
        },
    }
}

/// Result of compiling for HMR.
#[derive(Default)]
#[napi(object)]
pub struct HmrCompileResult {
    /// The complete HMR update module code.
    pub hmr_module: String,

    /// The component ID (path@ClassName).
    pub component_id: String,

    /// The compiled template function as JavaScript.
    pub template_js: String,

    /// Compilation errors.
    pub errors: Vec<OxcError>,
}

/// Async task for HMR compilation.
pub struct CompileForHmrTask {
    template: String,
    component_name: String,
    file_path: String,
    styles: Option<Vec<String>>,
    options: TransformOptions,
}

#[napi]
impl Task for CompileForHmrTask {
    type JsValue = HmrCompileResult;
    type Output = HmrCompileResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(compile_for_hmr_sync(
            self.template.clone(),
            self.component_name.clone(),
            self.file_path.clone(),
            self.styles.clone(),
            Some(std::mem::take(&mut self.options)),
        ))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Compile for HMR (async).
///
/// This is the async version of `compileForHmrSync`.
#[napi]
pub fn compile_for_hmr(
    template: String,
    component_name: String,
    file_path: String,
    styles: Option<Vec<String>>,
    options: Option<TransformOptions>,
) -> AsyncTask<CompileForHmrTask> {
    AsyncTask::new(CompileForHmrTask {
        template,
        component_name,
        file_path,
        styles,
        options: options.unwrap_or_default(),
    })
}

/// URL-encode a component ID for use in import URLs.
///
/// # Arguments
///
/// * `component_id` - The component ID to encode
///
/// # Returns
///
/// URL-encoded component ID.
#[napi]
pub fn encode_component_id(component_id: String) -> String {
    component_id.replace('/', "%2F").replace('@', "%40").replace(' ', "%20")
}

/// Decode a URL-encoded component ID.
///
/// # Arguments
///
/// * `encoded_id` - The URL-encoded component ID
///
/// # Returns
///
/// Decoded component ID.
#[napi]
pub fn decode_component_id(encoded_id: String) -> String {
    encoded_id.replace("%2F", "/").replace("%40", "@").replace("%20", " ")
}

/// Parse a component ID into its parts.
///
/// # Arguments
///
/// * `component_id` - The component ID (path@ClassName)
///
/// # Returns
///
/// Object with `filePath` and `className` properties.
#[napi]
pub fn parse_component_id(component_id: String) -> ComponentIdParts {
    if let Some(idx) = component_id.rfind('@') {
        ComponentIdParts {
            file_path: component_id[..idx].to_string(),
            class_name: component_id[idx + 1..].to_string(),
        }
    } else {
        ComponentIdParts { file_path: component_id, class_name: String::new() }
    }
}

/// Parts of a component ID.
#[napi(object)]
pub struct ComponentIdParts {
    /// The file path part of the component ID.
    pub file_path: String,

    /// The class name part of the component ID.
    pub class_name: String,
}

/// Encapsulate CSS styles for a component using attribute selectors.
///
/// This implements Angular's ViewEncapsulation.Emulated behavior,
/// scoping CSS styles to a component by adding attribute selectors.
///
/// # Arguments
///
/// * `css` - The CSS source code to encapsulate
/// * `component_id` - The component's unique identifier (typically a hash)
///
/// # Returns
///
/// The CSS with all selectors scoped to the component.
///
/// # Example
///
/// Input:
/// ```css
/// .button { color: red; }
/// ```
///
/// Output (with component_id "abc123"):
/// ```css
/// .button[ng-cabc123] { color: red; }
/// ```
#[napi]
pub fn encapsulate_style(css: String, component_id: String) -> String {
    rust_encapsulate_style(&css, &component_id)
}

/// URLs extracted from @Component decorators in a file.
#[napi(object)]
pub struct ComponentUrls {
    /// Template URLs from templateUrl properties.
    pub template_urls: Vec<String>,
    /// Style URLs from styleUrl and styleUrls properties.
    pub style_urls: Vec<String>,
}

/// Extract templateUrl and styleUrls from all @Component decorators in a file.
///
/// This parses a TypeScript file and extracts URL references from all
/// @Component decorators found in the file.
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `filename` - The filename (used for source type detection)
///
/// # Returns
///
/// A `ComponentUrls` containing all template and style URLs found.
pub fn extract_component_urls_sync(source: String, filename: String) -> ComponentUrls {
    use oxc_angular_compiler::{build_import_map, extract_component_metadata};
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&filename).unwrap_or_default();

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    // Build import map for component metadata extraction
    let import_map = build_import_map(&allocator, &program.body, None);

    let mut template_urls = Vec::new();
    let mut style_urls = Vec::new();

    // Walk statements looking for class declarations
    for stmt in &program.body {
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

        if let Some(class) = class {
            // Extract metadata from @Component decorator
            // Use implicit_standalone=true (v19+ default) since it doesn't affect URL extraction
            if let Some(metadata) = extract_component_metadata(&allocator, class, true, &import_map)
            {
                // Collect template URL
                if let Some(template_url) = &metadata.template_url {
                    template_urls.push(template_url.to_string());
                }

                // Collect style URLs
                for url in &metadata.style_urls {
                    style_urls.push(url.to_string());
                }
            }
        }
    }

    ComponentUrls { template_urls, style_urls }
}

/// Async task for extracting component URLs.
pub struct ExtractComponentUrlsTask {
    source: String,
    filename: String,
}

#[napi]
impl Task for ExtractComponentUrlsTask {
    type JsValue = ComponentUrls;
    type Output = ComponentUrls;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(extract_component_urls_sync(self.source.clone(), self.filename.clone()))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Extract templateUrl and styleUrls from all @Component decorators in a file (async).
///
/// This is the async version of `extractComponentUrlsSync`. Use this when
/// extracting URLs in a non-blocking context.
#[napi]
pub fn extract_component_urls(
    source: String,
    filename: String,
) -> AsyncTask<ExtractComponentUrlsTask> {
    AsyncTask::new(ExtractComponentUrlsTask { source, filename })
}

/// Top-level declarations extracted from a TypeScript file for HMR.
///
/// These are the symbols that can be passed as local dependencies to
/// the HMR update function.
#[napi(object)]
pub struct TopLevelDeclarations {
    /// Names of all top-level declarations in the file.
    ///
    /// Includes: classes, functions, const enums, variables, and imports.
    /// Excludes: type-only imports, interfaces, type aliases.
    pub names: Vec<String>,
}

/// Extract top-level declaration names from a TypeScript file.
///
/// This implements the `getTopLevelDeclarationNames` function from Angular's
/// compiler-cli `extract_dependencies.ts`. It finds all top-level symbols that
/// could be referenced in component code and need to be passed to the HMR
/// update function.
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `filename` - The filename (used for source type detection)
///
/// # Returns
///
/// A `TopLevelDeclarations` containing all top-level declaration names.
///
/// # Example
///
/// For source code:
/// ```typescript
/// import { Component } from '@angular/core';
/// import * as utils from './utils';
/// import type { SomeType } from './types'; // excluded (type-only)
///
/// const config = { ... };
/// function helper() { ... }
///
/// @Component({ ... })
/// export class MyComponent { ... }
/// ```
///
/// Returns names: `["Component", "utils", "config", "helper", "MyComponent"]`
pub fn extract_top_level_declarations_sync(
    source: String,
    filename: String,
) -> TopLevelDeclarations {
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&filename).unwrap_or_default();

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    let mut names = Vec::new();

    // Process top-level statements
    for stmt in &program.body {
        match stmt {
            // Class declarations: class Foo { ... }
            Statement::ClassDeclaration(class) => {
                if let Some(id) = &class.id {
                    names.push(id.name.to_string());
                }
            }

            // Function declarations: function foo() { ... }
            Statement::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    names.push(id.name.to_string());
                }
            }

            // Enum declarations: enum Foo { ... } or const enum Foo { ... }
            Statement::TSEnumDeclaration(enum_decl) => {
                names.push(enum_decl.id.name.to_string());
            }

            // Variable declarations: const foo = ..., let bar = ...
            Statement::VariableDeclaration(var_decl) => {
                for decl in &var_decl.declarations {
                    track_binding_pattern_names(&decl.id, &mut names);
                }
            }

            // Export default: export default class Foo { ... }
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                    if let Some(id) = &class.id {
                        names.push(id.name.to_string());
                    }
                }
                ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                    if let Some(id) = &func.id {
                        names.push(id.name.to_string());
                    }
                }
                _ => {}
            },

            // Export named: export class Foo { ... }, export const foo = ...
            Statement::ExportNamedDeclaration(export) => {
                if let Some(decl) = &export.declaration {
                    match decl {
                        Declaration::ClassDeclaration(class) => {
                            if let Some(id) = &class.id {
                                names.push(id.name.to_string());
                            }
                        }
                        Declaration::FunctionDeclaration(func) => {
                            if let Some(id) = &func.id {
                                names.push(id.name.to_string());
                            }
                        }
                        Declaration::TSEnumDeclaration(enum_decl) => {
                            names.push(enum_decl.id.name.to_string());
                        }
                        Declaration::VariableDeclaration(var_decl) => {
                            for d in &var_decl.declarations {
                                track_binding_pattern_names(&d.id, &mut names);
                            }
                        }
                        _ => {}
                    }
                }
            }

            // Module declarations (imports)
            Statement::ImportDeclaration(import) => {
                // Skip type-only imports: import type { ... } from '...'
                if import.import_kind.is_type() {
                    continue;
                }

                if let Some(specifiers) = &import.specifiers {
                    for specifier in specifiers {
                        match specifier {
                            // import foo from 'foo'
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                                names.push(s.local.name.to_string());
                            }
                            // import * as foo from 'foo'
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportNamespaceSpecifier(
                                s,
                            ) => {
                                names.push(s.local.name.to_string());
                            }
                            // import { foo } from 'foo' or import { foo as bar } from 'foo'
                            oxc_ast::ast::ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                // Skip type-only specifiers
                                if !s.import_kind.is_type() {
                                    names.push(s.local.name.to_string());
                                }
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }

    TopLevelDeclarations { names }
}

/// Helper to extract names from binding patterns (destructuring).
fn track_binding_pattern_names(pattern: &oxc_ast::ast::BindingPattern, names: &mut Vec<String>) {
    use oxc_ast::ast::BindingPattern;

    match pattern {
        BindingPattern::BindingIdentifier(id) => {
            names.push(id.name.to_string());
        }
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                track_binding_pattern_names(&prop.value, names);
            }
            if let Some(rest) = &obj.rest {
                track_binding_pattern_names(&rest.argument, names);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for elem in arr.elements.iter().flatten() {
                track_binding_pattern_names(elem, names);
            }
            if let Some(rest) = &arr.rest {
                track_binding_pattern_names(&rest.argument, names);
            }
        }
        BindingPattern::AssignmentPattern(assign) => {
            track_binding_pattern_names(&assign.left, names);
        }
    }
}

/// Pre-resolved external resources for file transformation.
///
/// Note: NAPI-RS converts JavaScript plain objects (Record/object) to Rust HashMap,
/// NOT JavaScript Map objects. Pass plain objects, not `new Map()`.
#[derive(Default)]
#[napi(object)]
pub struct ResolvedResources {
    /// Map from templateUrl path to resolved template content.
    /// Pass as a plain object: `{ './template.html': 'content' }`, not `new Map()`.
    #[napi(ts_type = "Record<string, string>")]
    pub templates: HashMap<String, String>,

    /// Map from styleUrl path to resolved (preprocessed) style content.
    /// Pass as a plain object: `{ './styles.scss': ['compiled css'] }`, not `new Map()`.
    #[napi(ts_type = "Record<string, string[]>")]
    pub styles: HashMap<String, Vec<String>>,
}

/// Async task for full Angular file transformation.
pub struct TransformAngularFileTask {
    source: String,
    filename: String,
    options: TransformOptions,
    resolved_resources: Option<ResolvedResources>,
}

#[napi]
impl Task for TransformAngularFileTask {
    type JsValue = TransformResult;
    type Output = TransformResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        use oxc_angular_compiler::{
            ResolvedResources as RustResolvedResources, transform_angular_file,
        };

        let allocator = Allocator::default();
        let rust_options: RustTransformOptions = std::mem::take(&mut self.options).into();

        // Convert resolved resources to Rust types
        let rust_resources = self
            .resolved_resources
            .take()
            .map(|r| RustResolvedResources { templates: r.templates, styles: r.styles });

        let result = transform_angular_file(
            &allocator,
            &self.filename,
            &self.source,
            &rust_options,
            rust_resources.as_ref(),
        );

        // Convert diagnostics to errors (for simplicity, all diagnostics become errors)
        let errors = OxcError::from_diagnostics(&self.filename, &self.source, result.diagnostics);

        Ok(TransformResult {
            code: result.code,
            map: result.map,
            dependencies: result.dependencies,
            template_updates: result.template_updates,
            style_updates: result.style_updates,
            errors,
            warnings: vec![],
            dts_declarations: result
                .dts_declarations
                .into_iter()
                .map(|d| DtsDeclaration { class_name: d.class_name, members: d.members })
                .collect(),
        })
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Transform an Angular TypeScript file to JavaScript (async).
///
/// This performs the complete transformation pipeline:
/// 1. Parse the TypeScript file using oxc_parser
/// 2. Find @Component decorated classes
/// 3. Inline resolved templates and styles
/// 4. Compile templates to Angular IR code
/// 5. Generate JavaScript output using oxc_codegen
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `filename` - The file path (for source maps and error messages)
/// * `options` - Transformation options
/// * `resolved_resources` - Optional pre-resolved external resources
///
/// # Returns
///
/// A `TransformResult` containing the transformed code, dependencies, and any errors.
#[napi]
pub fn transform_angular_file(
    source: String,
    filename: String,
    options: TransformOptions,
    resolved_resources: Option<ResolvedResources>,
) -> AsyncTask<TransformAngularFileTask> {
    AsyncTask::new(TransformAngularFileTask { source, filename, options, resolved_resources })
}

#[napi]
pub fn transform_angular_file_sync(
    source: String,
    filename: String,
    options: TransformOptions,
    resolved_resources: Option<ResolvedResources>,
) -> napi::Result<TransformResult> {
    let mut result = TransformAngularFileTask { source, filename, options, resolved_resources };
    result.compute()
}

// =============================================================================
// Pipe Compilation
// =============================================================================

/// Extracted pipe metadata from a `@Pipe` decorator.
#[napi(object)]
pub struct ExtractedPipeMetadata {
    /// The name of the pipe class.
    pub class_name: String,
    /// Span start of the class declaration.
    pub span_start: u32,
    /// Span end of the class declaration.
    pub span_end: u32,
    /// The pipe name used in templates (from `@Pipe({name: '...'})`)
    pub pipe_name: Option<String>,
    /// Whether the pipe is pure (default: true).
    pub pure: bool,
    /// Whether this is a standalone pipe.
    pub standalone: bool,
}

/// Result of pipe compilation.
#[napi(object)]
pub struct PipeCompileResult {
    /// The compiled pipe definition as JavaScript code.
    pub code: String,
    /// Any errors that occurred during compilation.
    pub errors: Vec<OxcError>,
}

/// Extract pipe metadata from a TypeScript file.
///
/// This parses the file and extracts metadata from `@Pipe` decorators
/// on class declarations.
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `file_path` - The file path (for error messages)
/// * `pipe_name` - The name of the pipe class to extract
/// * `implicit_standalone` - Default value for standalone when not specified (true for Angular v19+)
///
/// # Returns
///
/// The extracted pipe metadata, or None if not found.
pub fn extract_pipe_metadata_sync(
    source: String,
    file_path: String,
    pipe_name: String,
    implicit_standalone: Option<bool>,
) -> Option<ExtractedPipeMetadata> {
    use oxc_angular_compiler::extract_pipe_metadata;
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    let implicit_standalone = implicit_standalone.unwrap_or(true);

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    // Find the pipe class by name
    for stmt in &program.body {
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

        if let Some(class) = class {
            // Check if this is the pipe we're looking for
            let Some(id) = &class.id else {
                continue;
            };
            if id.name.as_str() != pipe_name {
                continue;
            }

            // Extract metadata from @Pipe decorator
            if let Some(metadata) = extract_pipe_metadata(&allocator, class, implicit_standalone) {
                return Some(ExtractedPipeMetadata {
                    class_name: metadata.class_name.to_string(),
                    span_start: metadata.class_span.start,
                    span_end: metadata.class_span.end,
                    pipe_name: metadata.pipe_name.map(|n| n.to_string()),
                    pure: metadata.pure,
                    standalone: metadata.standalone,
                });
            }
        }
    }

    None
}

/// Compile a pipe from a TypeScript file.
///
/// This parses the file, extracts metadata from the `@Pipe` decorator,
/// and compiles it to JavaScript.
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `file_path` - The file path (for error messages)
/// * `pipe_name` - The name of the pipe class to compile
/// * `implicit_standalone` - Default value for standalone when not specified (true for Angular v19+)
///
/// # Returns
///
/// The compiled pipe definition code, or errors if compilation failed.
pub fn compile_pipe_sync(
    source: String,
    file_path: String,
    pipe_name: String,
    implicit_standalone: Option<bool>,
) -> PipeCompileResult {
    use oxc_angular_compiler::output::emitter::JsEmitter;
    use oxc_angular_compiler::{R3PipeMetadataBuilder, compile_pipe, extract_pipe_metadata};
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::{Atom, SourceType};

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    let implicit_standalone = implicit_standalone.unwrap_or(true);

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    // Find the pipe class by name
    for stmt in &program.body {
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

        if let Some(class) = class {
            // Check if this is the pipe we're looking for
            let Some(id) = &class.id else {
                continue;
            };
            if id.name.as_str() != pipe_name {
                continue;
            }

            // Extract metadata from @Pipe decorator
            if let Some(metadata) = extract_pipe_metadata(&allocator, class, implicit_standalone) {
                // Create type expression for the pipe class
                use oxc_allocator::Box;
                use oxc_angular_compiler::output::ast::{OutputExpression, ReadVarExpr};

                let type_expr = OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: metadata.class_name, source_span: None },
                    &allocator,
                ));

                // Build R3PipeMetadata
                let mut builder = R3PipeMetadataBuilder::new(metadata.class_name, type_expr)
                    .pure(metadata.pure)
                    .is_standalone(metadata.standalone);

                if let Some(name) = &metadata.pipe_name {
                    builder = builder.pipe_name(Atom::from(name.as_str()));
                }

                let r3_metadata = builder.build();
                let result = compile_pipe(&allocator, &r3_metadata);
                let emitter = JsEmitter::new();
                let code = emitter.emit_expression(&result.expression);

                return PipeCompileResult { code, errors: vec![] };
            }
        }
    }

    // Pipe not found
    PipeCompileResult {
        code: String::new(),
        errors: vec![OxcError::new(format!("Pipe '{pipe_name}' not found in file"))],
    }
}

// =============================================================================
// Component Metadata Extraction
// =============================================================================

/// Host metadata extracted from a `@Component` decorator.
#[napi(object)]
pub struct ComponentHostMetadata {
    /// Host property bindings: [[key, value], ...].
    pub properties: Vec<Vec<String>>,
    /// Host attribute bindings: [[key, value], ...].
    pub attributes: Vec<Vec<String>>,
    /// Host event listeners: [[key, value], ...].
    pub listeners: Vec<Vec<String>>,
    /// Static class attribute binding.
    pub class_attr: Option<String>,
    /// Static style attribute binding.
    pub style_attr: Option<String>,
}

/// Host directive metadata extracted from a `@Component` decorator.
#[napi(object)]
pub struct ExtractedHostDirective {
    /// The directive class name.
    pub directive: String,
    /// Input mappings: [[publicName, internalName], ...].
    pub inputs: Vec<Vec<String>>,
    /// Output mappings: [[publicName, internalName], ...].
    pub outputs: Vec<Vec<String>>,
    /// Whether this is a forward reference.
    pub is_forward_reference: bool,
}

/// Extracted input metadata from an `@Input` decorator.
#[napi(object)]
pub struct ExtractedInputMetadata {
    /// The property name on the class.
    pub class_property_name: String,
    /// The binding property name (can differ from class property name).
    pub binding_property_name: String,
    /// Whether this input is required.
    pub required: bool,
    /// Whether this is a signal-based input.
    pub is_signal: bool,
    /// Transform function expression as emitted JavaScript (if present).
    pub transform: Option<String>,
}

/// Extracted output metadata from an `@Output` decorator.
#[napi(object)]
pub struct ExtractedOutputMetadata {
    /// The property name on the class.
    pub class_property_name: String,
    /// The binding property name (can differ from class property name).
    pub binding_property_name: String,
}

/// Extracted query metadata from `@ViewChild`, `@ViewChildren`, `@ContentChild`, or `@ContentChildren` decorators.
#[napi(object)]
pub struct ExtractedQueryMetadata {
    /// The property name on the class.
    pub property_name: String,
    /// The query predicate (serialized as a string).
    pub predicate: String,
    /// Whether to include only direct children or all descendants.
    pub descendants: Option<bool>,
    /// Whether this query should collect only static results.
    #[napi(js_name = "static")]
    pub is_static: Option<bool>,
    /// An expression representing a type to read from each matched node.
    pub read: Option<String>,
    /// Whether this query returns only the first matching result (ViewChild/ContentChild vs ViewChildren/ContentChildren).
    pub first: bool,
}

/// Extracted component metadata from a `@Component` decorator.
#[napi(object)]
pub struct ExtractedComponentMetadata {
    /// The name of the component class.
    pub class_name: String,
    /// Span start of the class declaration.
    pub span_start: u32,
    /// Span end of the class declaration.
    pub span_end: u32,
    /// The CSS selector.
    pub selector: Option<String>,
    /// Inline template string.
    pub template: Option<String>,
    /// URL to an external template file.
    pub template_url: Option<String>,
    /// Inline styles array.
    pub styles: Vec<String>,
    /// URLs to external stylesheet files.
    pub style_urls: Vec<String>,
    /// Whether this is a standalone component.
    pub standalone: bool,
    /// View encapsulation mode: "Emulated" | "None" | "ShadowDom".
    pub encapsulation: String,
    /// Change detection strategy: "Default" | "OnPush".
    pub change_detection: String,
    /// Host bindings and listeners.
    pub host: Option<ComponentHostMetadata>,
    /// Component imports (for standalone components).
    pub imports: Vec<String>,
    /// Exported names for template references.
    pub export_as: Option<String>,
    /// Whether to preserve whitespace in templates.
    pub preserve_whitespaces: bool,
    /// Providers expression as emitted JavaScript (if present).
    pub providers: Option<String>,
    /// View providers expression as emitted JavaScript (if present).
    pub view_providers: Option<String>,
    /// Animations expression as emitted JavaScript (if present).
    pub animations: Option<String>,
    /// Schema identifiers.
    pub schemas: Vec<String>,
    /// Host directives configuration.
    pub host_directives: Vec<ExtractedHostDirective>,
    /// Inputs extracted from @Input decorators on class members.
    pub inputs: Option<Vec<ExtractedInputMetadata>>,
    /// Outputs extracted from @Output decorators on class members.
    pub outputs: Option<Vec<ExtractedOutputMetadata>>,
    /// Content queries extracted from @ContentChild/@ContentChildren decorators.
    pub queries: Option<Vec<ExtractedQueryMetadata>>,
    /// View queries extracted from @ViewChild/@ViewChildren decorators.
    pub view_queries: Option<Vec<ExtractedQueryMetadata>>,
}

/// Extract component metadata from all `@Component` decorated classes in a TypeScript file.
///
/// This parses the file and extracts metadata from `@Component` decorators
/// on class declarations.
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `file_path` - The file path (for error messages and source type detection)
///
/// # Returns
///
/// A vector of `ExtractedComponentMetadata` for each component found.
pub fn extract_component_metadata_sync(
    source: String,
    file_path: String,
) -> Vec<ExtractedComponentMetadata> {
    use oxc_angular_compiler::output::emitter::JsEmitter;
    use oxc_angular_compiler::{
        ChangeDetectionStrategy as RustChangeDetection, QueryPredicate,
        ViewEncapsulation as RustViewEncapsulation, build_import_map, extract_component_metadata,
        extract_content_queries, extract_input_metadata, extract_output_metadata,
        extract_view_queries,
    };
    use oxc_ast::ast::{Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();
    // Use implicit_standalone=true (v19+ default)
    let implicit_standalone = true;

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    // Build import map for component metadata extraction
    let import_map = build_import_map(&allocator, &program.body, None);

    let mut results = Vec::new();
    let emitter = JsEmitter::new();

    // Walk statements looking for class declarations
    for stmt in &program.body {
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

        if let Some(class) = class {
            // Extract metadata from @Component decorator
            if let Some(metadata) =
                extract_component_metadata(&allocator, class, implicit_standalone, &import_map)
            {
                // Convert encapsulation to string
                let encapsulation = match metadata.encapsulation {
                    RustViewEncapsulation::Emulated => "Emulated",
                    RustViewEncapsulation::None => "None",
                    RustViewEncapsulation::ShadowDom => "ShadowDom",
                }
                .to_string();

                // Convert change detection to string
                let change_detection = match metadata.change_detection {
                    RustChangeDetection::Default => "Default",
                    RustChangeDetection::OnPush => "OnPush",
                }
                .to_string();

                // Convert host metadata
                let host = metadata.host.as_ref().map(|h| ComponentHostMetadata {
                    properties: h
                        .properties
                        .iter()
                        .map(|(k, v)| vec![k.to_string(), v.to_string()])
                        .collect(),
                    attributes: h
                        .attributes
                        .iter()
                        .map(|(k, v)| vec![k.to_string(), v.to_string()])
                        .collect(),
                    listeners: h
                        .listeners
                        .iter()
                        .map(|(k, v)| vec![k.to_string(), v.to_string()])
                        .collect(),
                    class_attr: h.class_attr.as_ref().map(std::string::ToString::to_string),
                    style_attr: h.style_attr.as_ref().map(std::string::ToString::to_string),
                });

                // Convert host directives
                let host_directives: Vec<ExtractedHostDirective> = metadata
                    .host_directives
                    .iter()
                    .map(|hd| ExtractedHostDirective {
                        directive: hd.directive.to_string(),
                        inputs: hd
                            .inputs
                            .iter()
                            .map(|(k, v)| vec![k.to_string(), v.to_string()])
                            .collect(),
                        outputs: hd
                            .outputs
                            .iter()
                            .map(|(k, v)| vec![k.to_string(), v.to_string()])
                            .collect(),
                        is_forward_reference: hd.is_forward_reference,
                    })
                    .collect();

                // Convert providers/viewProviders/animations to JS strings
                let providers = metadata.providers.as_ref().map(|e| emitter.emit_expression(e));
                let view_providers =
                    metadata.view_providers.as_ref().map(|e| emitter.emit_expression(e));
                let animations = metadata.animations.as_ref().map(|e| emitter.emit_expression(e));

                // Extract inputs from @Input decorators
                let rust_inputs = extract_input_metadata(&allocator, class);
                let inputs: Option<Vec<ExtractedInputMetadata>> = if rust_inputs.is_empty() {
                    None
                } else {
                    Some(
                        rust_inputs
                            .iter()
                            .map(|input| ExtractedInputMetadata {
                                class_property_name: input.class_property_name.to_string(),
                                binding_property_name: input.binding_property_name.to_string(),
                                required: input.required,
                                is_signal: input.is_signal,
                                transform: input
                                    .transform_function
                                    .as_ref()
                                    .map(|e| emitter.emit_expression(e)),
                            })
                            .collect(),
                    )
                };

                // Extract outputs from @Output decorators
                let rust_outputs = extract_output_metadata(&allocator, class);
                let outputs: Option<Vec<ExtractedOutputMetadata>> = if rust_outputs.is_empty() {
                    None
                } else {
                    Some(
                        rust_outputs
                            .iter()
                            .map(|(class_name, binding_name)| ExtractedOutputMetadata {
                                class_property_name: class_name.to_string(),
                                binding_property_name: binding_name.to_string(),
                            })
                            .collect(),
                    )
                };

                // Helper function to serialize query predicate
                fn serialize_predicate(
                    predicate: &QueryPredicate<'_>,
                    emitter: &JsEmitter,
                ) -> String {
                    match predicate {
                        QueryPredicate::Type(expr) => emitter.emit_expression(expr),
                        QueryPredicate::Selectors(selectors) => {
                            // Serialize as JSON array of strings
                            let strings: Vec<String> =
                                selectors.iter().map(|s| format!("\"{s}\"")).collect();
                            format!("[{}]", strings.join(", "))
                        }
                    }
                }

                // Extract view queries from @ViewChild/@ViewChildren decorators
                let rust_view_queries = extract_view_queries(&allocator, class);
                let view_queries: Option<Vec<ExtractedQueryMetadata>> =
                    if rust_view_queries.is_empty() {
                        None
                    } else {
                        Some(
                            rust_view_queries
                                .iter()
                                .map(|q| ExtractedQueryMetadata {
                                    property_name: q.property_name.to_string(),
                                    predicate: serialize_predicate(&q.predicate, &emitter),
                                    descendants: Some(q.descendants),
                                    is_static: Some(q.is_static),
                                    read: q.read.as_ref().map(|e| emitter.emit_expression(e)),
                                    first: q.first,
                                })
                                .collect(),
                        )
                    };

                // Extract content queries from @ContentChild/@ContentChildren decorators
                let rust_content_queries = extract_content_queries(&allocator, class);
                let queries: Option<Vec<ExtractedQueryMetadata>> =
                    if rust_content_queries.is_empty() {
                        None
                    } else {
                        Some(
                            rust_content_queries
                                .iter()
                                .map(|q| ExtractedQueryMetadata {
                                    property_name: q.property_name.to_string(),
                                    predicate: serialize_predicate(&q.predicate, &emitter),
                                    descendants: Some(q.descendants),
                                    is_static: Some(q.is_static),
                                    read: q.read.as_ref().map(|e| emitter.emit_expression(e)),
                                    first: q.first,
                                })
                                .collect(),
                        )
                    };

                results.push(ExtractedComponentMetadata {
                    class_name: metadata.class_name.to_string(),
                    span_start: metadata.class_span.start,
                    span_end: metadata.class_span.end,
                    selector: metadata.selector.as_ref().map(std::string::ToString::to_string),
                    template: metadata.template.as_ref().map(std::string::ToString::to_string),
                    template_url: metadata
                        .template_url
                        .as_ref()
                        .map(std::string::ToString::to_string),
                    styles: metadata.styles.iter().map(std::string::ToString::to_string).collect(),
                    style_urls: metadata
                        .style_urls
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                    standalone: metadata.standalone,
                    encapsulation,
                    change_detection,
                    host,
                    imports: metadata
                        .imports
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                    export_as: if metadata.export_as.is_empty() {
                        None
                    } else {
                        Some(
                            metadata
                                .export_as
                                .iter()
                                .map(oxc_span::Atom::as_str)
                                .collect::<Vec<_>>()
                                .join(","),
                        )
                    },
                    preserve_whitespaces: metadata.preserve_whitespaces,
                    providers,
                    view_providers,
                    animations,
                    schemas: metadata
                        .schemas
                        .iter()
                        .map(std::string::ToString::to_string)
                        .collect(),
                    host_directives,
                    inputs,
                    outputs,
                    queries,
                    view_queries,
                });
            }
        }
    }

    results
}

// =============================================================================
// Injector Compilation
// =============================================================================

/// Input for compiling an injector.
///
/// Injectors are generated from `@NgModule` metadata, not from a decorator.
/// This struct captures the metadata needed to compile an injector definition.
#[derive(Default)]
#[napi(object)]
pub struct InjectorCompileInput {
    /// The name of the injector (typically the NgModule class name).
    pub name: String,

    /// Optional serialized JavaScript expression for providers.
    /// Example: `"[MyService, { provide: TOKEN, useClass: MyImpl }]"`
    pub providers: Option<String>,

    /// Optional array of import class names.
    /// Example: `["CommonModule", "FormsModule"]`
    pub imports: Option<Vec<String>>,
}

/// Result of compiling an injector.
#[napi(object)]
pub struct InjectorNapiCompileResult {
    /// The compiled injector definition as JavaScript code.
    /// Example: `i0.ɵɵdefineInjector({ providers: [...], imports: [...] })`
    pub code: String,

    /// Compilation errors.
    pub errors: Vec<OxcError>,
}

/// Compile an injector from the provided metadata.
///
/// This generates an injector definition like:
/// ```javascript
/// i0.ɵɵdefineInjector({
///   providers: [...],
///   imports: [Module1, Module2]
/// })
/// ```
///
/// # Arguments
///
/// * `input` - The injector metadata (name, providers, imports)
///
/// # Returns
///
/// An `InjectorNapiCompileResult` containing the compiled code or errors.
///
/// # Note
///
/// The `providers` field should be a simple variable name that references
/// a providers array. For complex expressions, the caller should define
/// the expression as a variable first.
pub fn compile_injector_sync(input: InjectorCompileInput) -> InjectorNapiCompileResult {
    use oxc_allocator::Box;
    use oxc_angular_compiler::output::ast::{OutputExpression, ReadVarExpr};
    use oxc_angular_compiler::output::emitter::JsEmitter;
    use oxc_angular_compiler::{R3InjectorMetadataBuilder, compile_injector};
    use oxc_span::Atom;

    let allocator = Allocator::default();

    // Create type expression for the injector class
    let type_expr = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Atom::from(input.name.as_str()), source_span: None },
        &allocator,
    ));

    // Build the metadata
    let mut builder = R3InjectorMetadataBuilder::new(&allocator)
        .name(Atom::from(input.name.as_str()))
        .r#type(type_expr);

    // Add providers if present (as a variable reference)
    if let Some(providers_str) = &input.providers {
        let providers_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from(providers_str.as_str()), source_span: None },
            &allocator,
        ));
        builder = builder.providers(providers_expr);
    }

    // Add imports if present
    if let Some(imports) = &input.imports {
        for import_name in imports {
            let import_expr = OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Atom::from(import_name.as_str()), source_span: None },
                &allocator,
            ));
            builder = builder.add_import(import_expr);
        }
    }

    // Build and compile
    let Some(metadata) = builder.build() else {
        return InjectorNapiCompileResult {
            code: String::new(),
            errors: vec![OxcError::new(
                "Failed to build injector metadata: missing required fields".to_string(),
            )],
        };
    };

    let result = compile_injector(&allocator, &metadata);
    let emitter = JsEmitter::new();
    let code = emitter.emit_expression(&result.expression);

    InjectorNapiCompileResult { code, errors: vec![] }
}

// =============================================================================
// Class Metadata Compilation
// =============================================================================

/// Result of class metadata compilation.
#[napi(object)]
pub struct ClassMetadataNapiCompileResult {
    /// The compiled setClassMetadata call as JavaScript code.
    /// Example: `(() => { (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵɵsetClassMetadata(...); })()`
    pub code: String,

    /// Compilation errors.
    pub errors: Vec<OxcError>,
}

/// Compile class metadata for an Angular decorated class.
///
/// This generates a `setClassMetadata` call wrapped in an IIFE with ngDevMode guard:
/// ```javascript
/// (() => {
///   (typeof ngDevMode === "undefined" || ngDevMode) &&
///     i0.ɵɵsetClassMetadata(MyComponent, [...decorators], [...ctorParams], {...propDecorators});
/// })();
/// ```
///
/// The class metadata is used by Angular's TestBed to recompile components with overrides.
///
/// # Arguments
///
/// * `source` - The TypeScript source code
/// * `file_path` - The file path (for error messages)
/// * `class_name` - The name of the class to compile metadata for
/// * `decorator_type` - The decorator type: "Component", "Directive", "Pipe", "Injectable", "NgModule"
///
/// # Returns
///
/// A `ClassMetadataNapiCompileResult` containing the compiled code or errors.
pub fn compile_class_metadata_sync(
    source: String,
    file_path: String,
    class_name: String,
    decorator_type: String,
) -> ClassMetadataNapiCompileResult {
    use oxc_allocator::Box;
    use oxc_angular_compiler::class_metadata::{R3ClassMetadata, compile_class_metadata};
    use oxc_angular_compiler::output::ast::{OutputExpression, ReadVarExpr};
    use oxc_angular_compiler::output::emitter::JsEmitter;
    use oxc_ast::ast::{Class, Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_parser::Parser;
    use oxc_span::{Atom, SourceType};

    let allocator = Allocator::default();
    let source_type = SourceType::from_path(&file_path).unwrap_or_default();

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    // Find the class by name
    let mut found_class: Option<&Class<'_>> = None;
    for stmt in &program.body {
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

        if let Some(class) = class
            && let Some(id) = &class.id
            && id.name.as_str() == class_name
        {
            found_class = Some(class);
            break;
        }
    }

    let Some(class) = found_class else {
        return ClassMetadataNapiCompileResult {
            code: String::new(),
            errors: vec![OxcError::new(format!("Class '{class_name}' not found in file"))],
        };
    };

    // Find the target decorator on the class
    let target_decorator = find_angular_decorator(&class.decorators, &decorator_type);

    let Some(decorator) = target_decorator else {
        return ClassMetadataNapiCompileResult {
            code: String::new(),
            errors: vec![OxcError::new(format!(
                "@{decorator_type} decorator not found on class '{class_name}'"
            ))],
        };
    };

    // Build the class type expression
    let type_expr = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Atom::from(class_name.as_str()), source_span: None },
        &allocator,
    ));

    // Build decorators array: [{ type: DecoratorClass, args: [...] }]
    let decorator_ref = decorator;
    let decorators_expr = core_build_decorator_metadata_array(&allocator, &[decorator_ref]);

    // Build constructor parameters metadata
    // This standalone API doesn't have full transform pipeline context (constructor deps
    // and namespace registry), so imported types won't get namespace prefixes.
    // The full transform_angular_file pipeline handles namespace prefixes correctly.
    let mut namespace_registry = oxc_angular_compiler::NamespaceRegistry::new(&allocator);
    let empty_import_map = oxc_angular_compiler::ImportMap::default();
    let ctor_params_expr = core_build_ctor_params_metadata(
        &allocator,
        class,
        None,
        &mut namespace_registry,
        &empty_import_map,
    );

    // Build property decorators metadata
    let prop_decorators_expr = core_build_prop_decorators_metadata(&allocator, class);

    // Create R3ClassMetadata
    let metadata = R3ClassMetadata {
        r#type: type_expr,
        decorators: decorators_expr,
        ctor_parameters: ctor_params_expr,
        prop_decorators: prop_decorators_expr,
    };

    // Compile to output expression
    let result = compile_class_metadata(&allocator, &metadata);

    // Emit to JavaScript
    let emitter = JsEmitter::new();
    let code = emitter.emit_expression(&result);

    ClassMetadataNapiCompileResult { code, errors: vec![] }
}

/// Task for async class metadata compilation.
pub struct CompileClassMetadataTask {
    source: String,
    file_path: String,
    class_name: String,
    decorator_type: String,
}

#[napi]
impl Task for CompileClassMetadataTask {
    type JsValue = ClassMetadataNapiCompileResult;
    type Output = ClassMetadataNapiCompileResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(compile_class_metadata_sync(
            self.source.clone(),
            self.file_path.clone(),
            self.class_name.clone(),
            self.decorator_type.clone(),
        ))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Compile class metadata for an Angular decorated class (async).
///
/// This is the async version of `compileClassMetadataSync`. Use this when
/// compiling class metadata in a non-blocking context.
#[napi]
pub fn compile_class_metadata(
    source: String,
    file_path: String,
    class_name: String,
    decorator_type: String,
) -> AsyncTask<CompileClassMetadataTask> {
    AsyncTask::new(CompileClassMetadataTask { source, file_path, class_name, decorator_type })
}

/// Find an Angular decorator by name on a class.
fn find_angular_decorator<'a>(
    decorators: &'a [oxc_ast::ast::Decorator<'a>],
    decorator_name: &str,
) -> Option<&'a oxc_ast::ast::Decorator<'a>> {
    decorators.iter().find(|d| {
        match &d.expression {
            oxc_ast::ast::Expression::CallExpression(call) => match &call.callee {
                // Direct call: @Component()
                oxc_ast::ast::Expression::Identifier(id) => id.name == decorator_name,
                // Namespaced: @ng.Component() or @core.Component()
                oxc_ast::ast::Expression::StaticMemberExpression(member) => {
                    member.property.name == decorator_name
                }
                _ => false,
            },
            // Decorator without call: @Component (rare, usually invalid)
            oxc_ast::ast::Expression::Identifier(id) => id.name == decorator_name,
            _ => false,
        }
    })
}

// =============================================================================
// Factory Compilation
// =============================================================================

/// Dependency metadata for factory injection.
///
/// Describes a constructor parameter that needs to be injected.
#[derive(Default)]
#[napi(object)]
pub struct DependencyMetadata {
    /// The token expression as a JavaScript string (e.g., "SomeService", "SOME_TOKEN").
    /// If None, the dependency is invalid and will generate `invalidFactoryDep(index)`.
    pub token: Option<String>,

    /// If this is an @Attribute injection, the literal attribute name type.
    /// Otherwise None for regular injection.
    pub attribute_name_type: Option<String>,

    /// Whether this dependency has an @Host qualifier.
    pub host: Option<bool>,

    /// Whether this dependency has an @Optional qualifier.
    pub optional: Option<bool>,

    /// Whether this dependency has an @Self qualifier.
    pub self_: Option<bool>,

    /// Whether this dependency has an @SkipSelf qualifier.
    pub skip_self: Option<bool>,
}

/// Input for compiling a factory function.
///
/// Factory functions are generated as part of directive, component, pipe,
/// injectable, and NgModule compilation. This allows direct compilation
/// of a factory function from metadata.
#[derive(Default)]
#[napi(object)]
pub struct FactoryCompileInput {
    /// The name of the class for which to generate the factory.
    pub name: String,

    /// The target type: "Component" | "Directive" | "Injectable" | "Pipe" | "NgModule".
    /// Defaults to "Injectable" if not specified.
    pub target: Option<String>,

    /// The kind of dependencies: "Valid" | "Invalid" | "None".
    /// - "Valid": Normal dependencies that can be injected.
    /// - "Invalid": One or more dependencies couldn't be resolved (generates invalidFactory).
    /// - "None": No constructor, uses inherited factory pattern.
    /// Defaults to "Valid" if not specified.
    pub deps_kind: Option<String>,

    /// Dependencies to inject in the constructor.
    /// Only used when deps_kind is "Valid".
    pub deps: Option<Vec<DependencyMetadata>>,
}

/// Result of compiling a factory function.
#[napi(object)]
pub struct FactoryNapiCompileResult {
    /// The compiled factory function as JavaScript code.
    /// Example: `function MyClass_Factory(__ngFactoryType__) { return new ... }`
    pub code: String,

    /// Compilation errors.
    pub errors: Vec<OxcError>,
}

/// Compile a factory function from the provided metadata (internal sync implementation).
fn compile_factory_impl(input: FactoryCompileInput) -> FactoryNapiCompileResult {
    use oxc_allocator::{Box, Vec as AllocVec};
    use oxc_angular_compiler::factory::{
        FactoryTarget as RustFactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata,
        R3FactoryDeps, R3FactoryMetadata, compile_factory_function,
    };
    use oxc_angular_compiler::output::ast::{OutputExpression, ReadVarExpr};
    use oxc_angular_compiler::output::emitter::JsEmitter;
    use oxc_span::Atom;

    let allocator = Allocator::default();

    // Parse target type
    let target = match input.target.as_deref() {
        Some("Component") => RustFactoryTarget::Component,
        Some("Directive") => RustFactoryTarget::Directive,
        Some("Injectable") | None => RustFactoryTarget::Injectable,
        Some("Pipe") => RustFactoryTarget::Pipe,
        Some("NgModule") => RustFactoryTarget::NgModule,
        Some(other) => {
            return FactoryNapiCompileResult {
                code: String::new(),
                errors: vec![OxcError::new(format!(
                    "Invalid factory target '{other}'. Valid values: Component, Directive, Injectable, Pipe, NgModule"
                ))],
            };
        }
    };

    // Create type expression for the class
    let type_expr = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Atom::from(input.name.as_str()), source_span: None },
        &allocator,
    ));

    // Parse deps_kind and build deps
    let deps = match input.deps_kind.as_deref() {
        Some("Invalid") => R3FactoryDeps::Invalid,
        Some("None") => R3FactoryDeps::None,
        Some("Valid") | None => {
            // Build valid dependencies
            let mut dep_list = AllocVec::new_in(&allocator);
            if let Some(deps) = &input.deps {
                for dep in deps {
                    // Use ReadVarExpr for token since WrappedNodeExpr cannot be emitted
                    let token = dep.token.as_ref().map(|t| {
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from(t.as_str()), source_span: None },
                            &allocator,
                        ))
                    });

                    // Use ReadVarExpr for attribute name type
                    let attribute_name_type = dep.attribute_name_type.as_ref().map(|a| {
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from(a.as_str()), source_span: None },
                            &allocator,
                        ))
                    });

                    dep_list.push(R3DependencyMetadata {
                        token,
                        attribute_name_type,
                        host: dep.host.unwrap_or(false),
                        optional: dep.optional.unwrap_or(false),
                        self_: dep.self_.unwrap_or(false),
                        skip_self: dep.skip_self.unwrap_or(false),
                    });
                }
            }
            R3FactoryDeps::Valid(dep_list)
        }
        Some(other) => {
            return FactoryNapiCompileResult {
                code: String::new(),
                errors: vec![OxcError::new(format!(
                    "Invalid deps_kind '{other}'. Valid values: Valid, Invalid, None"
                ))],
            };
        }
    };

    // Build factory metadata
    let factory_name = format!("{}_Factory", input.name);
    let metadata = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: Atom::from(input.name.as_str()),
        type_expr: type_expr.clone_in(&allocator),
        type_decl: type_expr,
        type_argument_count: 0,
        deps,
        target,
    });

    // Compile and emit
    let result = compile_factory_function(&allocator, &metadata, &factory_name);
    let emitter = JsEmitter::new();
    let code = emitter.emit_expression(&result.expression);

    FactoryNapiCompileResult { code, errors: vec![] }
}

/// Async task for factory compilation.
pub struct CompileFactoryTask {
    input: FactoryCompileInput,
}

#[napi]
impl Task for CompileFactoryTask {
    type JsValue = FactoryNapiCompileResult;
    type Output = FactoryNapiCompileResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(compile_factory_impl(std::mem::take(&mut self.input)))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Compile a factory function from the provided metadata (async).
///
/// This generates a factory function that creates instances of the class
/// with proper dependency injection. The output format depends on the
/// deps_kind parameter.
///
/// # Arguments
///
/// * `input` - The factory metadata (name, target, deps)
///
/// # Returns
///
/// A `FactoryNapiCompileResult` containing the compiled code or errors.
#[napi]
pub fn compile_factory(input: FactoryCompileInput) -> AsyncTask<CompileFactoryTask> {
    AsyncTask::new(CompileFactoryTask { input })
}

// =============================================================================
// AST-based Component Extraction
// =============================================================================

/// Result of extracting Angular component definitions from compiled JavaScript.
#[napi(object)]
pub struct ComponentExtractionResult {
    /// Const declarations like `const _c0 = [...]`, `const _c1 = [...]`, etc.
    pub consts: Vec<String>,

    /// Template functions like `function ClassName_Template(rf, ctx) {...}`.
    pub template_functions: Vec<String>,

    /// Component definition like `ClassName.ɵcmp = defineComponent(...)`.
    pub component_def: Option<String>,

    /// Factory function like `ClassName.ɵfac = function(t) {...}`.
    pub factory_def: Option<String>,
}

/// Extract Angular component definitions from compiled JavaScript output using AST parsing.
///
/// This function parses compiled Angular JavaScript and extracts specific elements
/// for a given class name:
/// - Const declarations matching `_c\d+` pattern
/// - Template functions matching `{ClassName}_*_Template`
/// - Component definition (`ClassName.ɵcmp = defineComponent(...)`)
/// - Factory function (`ClassName.ɵfac = function(t) {...}`)
///
/// # Arguments
///
/// * `source` - The compiled JavaScript source code
/// * `class_name` - The name of the component class to extract
///
/// # Returns
///
/// A `ComponentExtractionResult` containing all found elements.
#[napi]
pub fn extract_angular_component_by_ast(
    source: String,
    class_name: String,
) -> ComponentExtractionResult {
    use oxc_ast::ast::{
        AssignmentTarget, ClassElement, Expression, PropertyKey, Statement, VariableDeclarationKind,
    };
    use oxc_parser::Parser;
    use oxc_span::{GetSpan, SourceType};
    use std::collections::HashSet;

    /// Check if a name matches the _c\d+ pattern (e.g., _c0, _c1, _c123)
    fn is_const_pattern(name: &str) -> bool {
        if !name.starts_with("_c") {
            return false;
        }
        let rest = &name[2..];
        !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
    }

    let allocator = Allocator::default();
    // Parse as JavaScript module
    let source_type = SourceType::mjs();

    let parser_ret = Parser::new(&allocator, &source, source_type).parse();
    let program = &parser_ret.program;

    let mut consts: Vec<String> = Vec::new();
    let mut template_functions: Vec<String> = Vec::new();
    let mut component_def: Option<String> = None;
    let mut factory_def: Option<String> = None;

    // Track seen const names to avoid duplicates
    let mut seen_consts: HashSet<String> = HashSet::new();

    // Walk through all statements
    for stmt in &program.body {
        match stmt {
            // Handle variable declarations: const _c0 = [...], const _c1 = [...]
            Statement::VariableDeclaration(var_decl) => {
                // Only process const declarations
                if var_decl.kind != VariableDeclarationKind::Const {
                    continue;
                }

                for declarator in &var_decl.declarations {
                    // Get the variable name from the binding identifier
                    if let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &declarator.id {
                        let name = id.name.as_str();
                        // Check if it matches _c\d+ pattern
                        if is_const_pattern(name) && !seen_consts.contains(name) {
                            seen_consts.insert(name.to_string());
                            // Extract the full declaration using span
                            let span = var_decl.span;
                            let code = &source[span.start as usize..span.end as usize];
                            consts.push(code.to_string());
                        }
                    }
                }
            }

            // Handle function declarations: function ClassName_Template(rf, ctx) {...}
            Statement::FunctionDeclaration(func) => {
                if let Some(id) = &func.id {
                    let name = id.name.as_str();
                    // Check if function name starts with ClassName_ and ends with _Template
                    if name.starts_with(&format!("{class_name}_")) && name.ends_with("_Template") {
                        let span = func.span;
                        let code = &source[span.start as usize..span.end as usize];
                        template_functions.push(code.to_string());
                    }
                }
            }

            // Handle expression statements: ClassName.ɵcmp = ..., ClassName.ɵfac = ...
            Statement::ExpressionStatement(expr_stmt) => {
                if let Expression::AssignmentExpression(assign) = &expr_stmt.expression {
                    // Check the left side for ClassName.ɵcmp or ClassName.ɵfac
                    if let AssignmentTarget::StaticMemberExpression(member) = &assign.left {
                        // Check if the object is the class name
                        let is_target_class = match &member.object {
                            Expression::Identifier(id) => id.name.as_str() == class_name,
                            _ => false,
                        };

                        if is_target_class {
                            let prop_name = member.property.name.as_str();
                            if prop_name == "ɵcmp" && component_def.is_none() {
                                let span = expr_stmt.span;
                                let code = &source[span.start as usize..span.end as usize];
                                // Remove trailing semicolon if present
                                let code = code.trim_end_matches(';').to_string();
                                component_def = Some(code);
                            } else if prop_name == "ɵfac" && factory_def.is_none() {
                                let span = expr_stmt.span;
                                let code = &source[span.start as usize..span.end as usize];
                                // Remove trailing semicolon if present
                                let code = code.trim_end_matches(';').to_string();
                                factory_def = Some(code);
                            }
                        }
                    }
                }
            }

            // Handle class declarations for static properties
            Statement::ClassDeclaration(class) => {
                // Check if this is the target class
                let is_target_class =
                    class.id.as_ref().is_some_and(|id| id.name.as_str() == class_name);

                if is_target_class {
                    for element in &class.body.body {
                        if let ClassElement::PropertyDefinition(prop) = element {
                            // Check if it's a static property
                            if prop.r#static {
                                let prop_name = match &prop.key {
                                    PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                                    _ => None,
                                };

                                if let Some(name) = prop_name {
                                    if name == "ɵcmp" && component_def.is_none() {
                                        // For static properties, we need to construct the assignment
                                        if let Some(value) = &prop.value {
                                            let value_span = value.span();
                                            let value_code = &source[value_span.start as usize
                                                ..value_span.end as usize];
                                            component_def =
                                                Some(format!("{class_name}.ɵcmp = {value_code}"));
                                        }
                                    } else if name == "ɵfac"
                                        && factory_def.is_none()
                                        && let Some(value) = &prop.value
                                    {
                                        let value_span = value.span();
                                        let value_code = &source
                                            [value_span.start as usize..value_span.end as usize];
                                        factory_def =
                                            Some(format!("{class_name}.ɵfac = {value_code}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Handle export declarations that might contain classes
            Statement::ExportNamedDeclaration(export) => {
                if let Some(oxc_ast::ast::Declaration::ClassDeclaration(class)) =
                    &export.declaration
                {
                    let is_target_class =
                        class.id.as_ref().is_some_and(|id| id.name.as_str() == class_name);

                    if is_target_class {
                        for element in &class.body.body {
                            if let ClassElement::PropertyDefinition(prop) = element
                                && prop.r#static
                            {
                                let prop_name = match &prop.key {
                                    PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                                    _ => None,
                                };

                                if let Some(name) = prop_name {
                                    if name == "ɵcmp" && component_def.is_none() {
                                        if let Some(value) = &prop.value {
                                            let value_span = value.span();
                                            let value_code = &source[value_span.start as usize
                                                ..value_span.end as usize];
                                            component_def =
                                                Some(format!("{class_name}.ɵcmp = {value_code}"));
                                        }
                                    } else if name == "ɵfac"
                                        && factory_def.is_none()
                                        && let Some(value) = &prop.value
                                    {
                                        let value_span = value.span();
                                        let value_code = &source
                                            [value_span.start as usize..value_span.end as usize];
                                        factory_def =
                                            Some(format!("{class_name}.ɵfac = {value_code}"));
                                    }
                                }
                            }
                        }
                    }
                }
            }

            _ => {}
        }
    }

    ComponentExtractionResult { consts, template_functions, component_def, factory_def }
}

// =============================================================================
// Build Optimizer
// =============================================================================

/// Options for the Angular build optimizer.
///
/// The optimizer applies transformations to pre-compiled Angular packages
/// (from node_modules/@angular/*) to enable better tree-shaking.
#[derive(Default)]
#[napi(object)]
pub struct OptimizeOptions {
    /// Generate source maps.
    pub sourcemap: Option<bool>,

    /// Remove Angular metadata calls (`ɵsetClassMetadata`, etc.).
    /// Default: true
    pub elide_metadata: Option<bool>,

    /// Wrap Angular static members in pure IIFEs for tree-shaking.
    /// Default: true
    pub wrap_static_members: Option<bool>,

    /// Add `@__PURE__` annotations to top-level calls.
    /// Default: true
    pub mark_pure: Option<bool>,

    /// Optimize TypeScript enum patterns to IIFEs.
    /// Default: true
    pub adjust_enums: Option<bool>,
}

/// Result of optimizing an Angular package file.
#[derive(Default)]
#[napi(object)]
pub struct OptimizeResult {
    /// The optimized code.
    pub code: String,

    /// Source map (if sourcemap option was enabled).
    pub map: Option<String>,
}

/// Optimize an Angular package file for better tree-shaking.
///
/// This function applies the Angular build optimizer transformations to
/// pre-compiled Angular packages. It parses the JavaScript, applies the
/// enabled transformations, and generates optimized output.
///
/// ## Transformations
///
/// The optimizer applies 4 transformations:
///
/// 1. **Elide Metadata**: Remove `ɵsetClassMetadata()`, `ɵsetClassMetadataAsync()`,
///    `ɵsetClassDebugInfo()` calls
/// 2. **Adjust Static Members**: Wrap static fields (`ɵcmp`, `ɵdir`, `ɵfac`, etc.)
///    in pure IIFE for tree-shaking
/// 3. **Mark Top Level Pure**: Add `@__PURE__` annotations to top-level calls
///    (except tslib/babel helpers)
/// 4. **Adjust TypeScript Enums**: Optimize TS enum patterns to IIFEs
///
/// # Arguments
///
/// * `code` - The JavaScript source code to optimize
/// * `filename` - The filename (used for source maps and error messages)
/// * `options` - Optimization options
///
/// # Returns
///
/// An `OptimizeResult` containing the optimized code and optional source map.
///
/// # Example
///
/// ```typescript
/// import { optimizeAngularPackageSync } from '@oxc-angular/vite';
///
/// const result = optimizeAngularPackageSync(
///   `let MyComponent = class MyComponent {};
///    MyComponent.ɵcmp = ɵɵdefineComponent({...});`,
///   'component.js',
///   { elideMetadata: true, wrapStaticMembers: true }
/// );
/// ```
pub fn optimize_angular_package_sync(
    code: String,
    filename: String,
    options: OptimizeOptions,
) -> OptimizeResult {
    use oxc_angular_compiler::optimizer::{OptimizeOptions as RustOptimizeOptions, optimize};

    let allocator = Allocator::default();

    // Convert NAPI options to Rust options
    let rust_options = RustOptimizeOptions {
        sourcemap: options.sourcemap.unwrap_or(false),
        elide_metadata: options.elide_metadata.unwrap_or(true),
        wrap_static_members: options.wrap_static_members.unwrap_or(true),
        mark_pure: options.mark_pure.unwrap_or(true),
        adjust_enums: options.adjust_enums.unwrap_or(true),
    };

    let result = optimize(&allocator, &code, &filename, rust_options);

    OptimizeResult { code: result.code, map: result.map }
}

/// Async task for build optimization.
pub struct OptimizeAngularPackageTask {
    code: String,
    filename: String,
    options: OptimizeOptions,
}

#[napi]
impl Task for OptimizeAngularPackageTask {
    type JsValue = OptimizeResult;
    type Output = OptimizeResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(optimize_angular_package_sync(
            std::mem::take(&mut self.code),
            std::mem::take(&mut self.filename),
            std::mem::take(&mut self.options),
        ))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Optimize an Angular package file for better tree-shaking (async).
///
/// This is the async version of `optimizeAngularPackageSync`. Use this when
/// optimizing packages in a non-blocking context.
#[napi]
pub fn optimize_angular_package(
    code: String,
    filename: String,
    options: OptimizeOptions,
) -> AsyncTask<OptimizeAngularPackageTask> {
    AsyncTask::new(OptimizeAngularPackageTask { code, filename, options })
}

// =============================================================================
// Angular Partial Declaration Linker
// =============================================================================

/// Result of linking Angular partial declarations.
#[napi(object)]
pub struct LinkResult {
    /// The linked code.
    pub code: String,
    /// Source map (if enabled).
    pub map: Option<String>,
    /// Whether any declarations were linked.
    pub linked: bool,
}

/// Link Angular partial declarations in a JavaScript file (sync).
///
/// Processes pre-compiled Angular library code containing `ɵɵngDeclare*` calls
/// and converts them to their fully compiled equivalents (`ɵɵdefine*` calls).
///
/// This is necessary for Angular libraries published with partial compilation
/// (Angular 12+). Without linking, Angular falls back to JIT compilation
/// which requires `@angular/compiler` at runtime.
///
/// # Arguments
///
/// * `code` - The JavaScript source code to link
/// * `filename` - The filename (for source maps and error messages)
///
/// # Returns
///
/// A `LinkResult` containing the linked code. If no partial declarations
/// were found, the original code is returned with `linked: false`.
#[napi]
pub fn link_angular_package_sync(code: String, filename: String) -> LinkResult {
    use oxc_angular_compiler::linker::link;

    let allocator = Allocator::default();
    let result = link(&allocator, &code, &filename);

    LinkResult { code: result.code, map: result.map, linked: result.linked }
}

/// Async task for Angular linking.
pub struct LinkAngularPackageTask {
    code: String,
    filename: String,
}

#[napi]
impl Task for LinkAngularPackageTask {
    type JsValue = LinkResult;
    type Output = LinkResult;

    fn compute(&mut self) -> napi::Result<Self::Output> {
        Ok(link_angular_package_sync(
            std::mem::take(&mut self.code),
            std::mem::take(&mut self.filename),
        ))
    }

    fn resolve(&mut self, _: napi::Env, result: Self::Output) -> napi::Result<Self::JsValue> {
        Ok(result)
    }
}

/// Link Angular partial declarations in a JavaScript file (async).
///
/// This is the async version of `linkAngularPackageSync`. Use this when
/// linking packages in a non-blocking context.
#[napi]
pub fn link_angular_package(code: String, filename: String) -> AsyncTask<LinkAngularPackageTask> {
    AsyncTask::new(LinkAngularPackageTask { code, filename })
}

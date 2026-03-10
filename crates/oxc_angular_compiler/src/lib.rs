//! Angular Template Compiler for Oxc
//!
//! This crate provides a Rust port of Angular's template compiler,
//! leveraging Oxc's infrastructure for memory management and code generation.
//!
//! ## Architecture
//!
//! The Angular compiler processes templates through a 6-stage pipeline:
//!
//! 1. **Parsing**: Template string → HTML AST via `HtmlParser`
//! 2. **Template Transform**: HTML AST → R3 AST via `htmlAstToRender3Ast`
//! 3. **Ingestion**: R3 AST → IR Operations
//! 4. **Transformation**: 67 ordered phases mutating IR
//! 5. **Emission**: IR → Output AST
//! 6. **Code Generation**: Output AST → JavaScript
//!
//! ## Build Tool Integration
//!
//! For integrating with build tools like Vite, use the [`component`] module:
//!
//! - [`component::extract_component_metadata`] - Extract metadata from `@Component` decorators
//! - [`component::compile_template_to_js`] - Compile a template to JavaScript
//! - [`component::transform_angular_file`] - Transform an entire Angular file

#![warn(missing_docs)]

mod util;

pub mod ast;
pub mod class_debug_info;
pub mod class_metadata;
pub mod component;
pub mod directive;
pub mod dts;
pub mod factory;
pub mod hmr;
pub mod i18n;
pub mod injectable;
pub mod injector;
pub mod ir;
pub mod linker;
pub mod ng_module;
pub mod optimizer;
pub mod output;
pub mod parser;
pub mod pipe;
pub mod pipeline;
pub mod r3;
pub mod schema;
pub mod styles;
pub mod transform;

// Re-export key types
pub use ast::expression::AngularExpression;
pub use ast::r3::R3Node;
pub use transform::{HtmlToR3Transform, html_to_r3::html_ast_to_r3_ast};

// Re-export component module types for convenience
pub use component::{
    AngularVersion, ChangeDetectionStrategy, CompiledComponent, ComponentMetadata,
    HmrTemplateCompileOutput, HostMetadata, HostMetadataInput, ImportInfo, ImportMap,
    NamespaceRegistry, ResolvedResources, TemplateCompileOutput, TransformOptions, TransformResult,
    ViewEncapsulation, build_import_map, compile_component_template, compile_for_hmr,
    compile_template_for_hmr, compile_template_to_js, compile_template_to_js_with_options,
    extract_component_metadata, transform_angular_file,
};

// Re-export cross-file elision types when feature is enabled
#[cfg(feature = "cross_file_elision")]
pub use component::CrossFileAnalyzer;

// Re-export HMR types
pub use hmr::{
    HmrDefinition, HmrDependencies, HmrLocalDependency, HmrMetadata, HmrNamespaceDependency,
    HmrUpdateModuleOptions, compile_hmr_initializer, compile_hmr_update_callback,
    extract_hmr_dependencies, generate_hmr_update_module, generate_hmr_update_module_from_js,
    generate_style_update_module,
};

// Re-export styles
pub use styles::{encapsulate_style, shim_css_text};

// Re-export pipe types
pub use pipe::{
    PipeCompileResult, PipeMetadata, R3DependencyMetadata, R3PipeMetadata, R3PipeMetadataBuilder,
    compile_pipe, compile_pipe_from_metadata, extract_pipe_metadata, find_pipe_decorator_span,
};

// Re-export factory types
pub use factory::{
    FactoryCompileResult, FactoryTarget, InjectFlags, R3ConstructorFactoryMetadata, R3FactoryDeps,
    R3FactoryMetadata, compile_factory_function,
};

// Re-export directive types
pub use directive::{
    DirectiveCompileResult, DirectiveDefinitions, QueryPredicate, R3DirectiveMetadata,
    R3DirectiveMetadataBuilder, R3HostDirectiveMetadata, R3HostMetadata, R3InputMetadata,
    R3QueryMetadata, compile_directive, compile_directive_from_metadata, extract_content_queries,
    extract_directive_metadata, extract_host_bindings, extract_host_listeners,
    extract_input_metadata, extract_output_metadata, extract_view_queries,
    find_directive_decorator_span, generate_directive_definitions,
};

// Re-export injectable types
pub use injectable::{
    InjectableCompileResult, InjectableDefinition, InjectableMetadata, InjectableProvider,
    R3InjectableMetadata, R3InjectableMetadataBuilder, compile_injectable,
    compile_injectable_from_metadata, extract_injectable_metadata, find_injectable_decorator_span,
    generate_injectable_definition, generate_injectable_definition_from_decorator,
};

// Re-export ng_module types
pub use ng_module::{
    NgModuleCompileResult, NgModuleDefinition, NgModuleMetadata, R3NgModuleMetadata,
    R3NgModuleMetadataBuilder, R3Reference, R3SelectorScopeMode, compile_ng_module,
    compile_ng_module_from_metadata, emit_ng_module_definition, extract_ng_module_metadata,
    find_ng_module_decorator_span, generate_ng_module_definition,
    generate_ng_module_definition_from_decorator,
};

// Re-export injector types
pub use injector::{
    InjectorCompileResult, R3InjectorMetadata, R3InjectorMetadataBuilder, compile_injector,
    compile_injector_from_metadata,
};

// Re-export class debug info types
pub use class_debug_info::{R3ClassDebugInfo, compile_class_debug_info};

// Re-export class metadata types
pub use class_metadata::{
    R3ClassMetadata, R3DeferPerComponentDependency, build_ctor_params_metadata,
    build_decorator_metadata_array, build_prop_decorators_metadata, compile_class_metadata,
    compile_component_class_metadata, compile_component_metadata_async_resolver,
    compile_opaque_async_class_metadata,
};

// Re-export dts types
pub use dts::{
    DtsDeclaration, generate_component_dts, generate_directive_dts, generate_injectable_dts,
    generate_ng_module_dts, generate_pipe_dts,
};

// Re-export linker types
pub use linker::{LinkResult, link};

// Re-export optimizer types
pub use optimizer::{OptimizeOptions, OptimizeResult, optimize};

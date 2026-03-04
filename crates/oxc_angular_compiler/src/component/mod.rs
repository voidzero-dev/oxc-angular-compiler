//! Component metadata extraction and transformation.
//!
//! This module provides functionality to:
//! - Extract `@Component` decorator metadata from TypeScript classes
//! - Generate ɵcmp/ɵfac component definitions
//! - Transform Angular component files for build tool integration
//! - Support HMR (Hot Module Replacement) workflows

#[cfg(feature = "cross_file_elision")]
mod cross_file_elision;
mod decorator;
mod definition;
mod dependency;
mod import_elision;
mod metadata;
mod namespace_registry;
mod transform;

#[cfg(feature = "cross_file_elision")]
pub use cross_file_elision::CrossFileAnalyzer;
pub use decorator::extract_component_metadata;
pub use definition::{
    ComponentDefinitions, const_value_to_expression, generate_component_definitions,
};
pub use dependency::{
    FactoryTarget, InjectFlags, R3DependencyMetadata, compile_inject_dependencies,
    generate_factory_di_args, get_inject_fn_name,
};
pub use import_elision::ImportElisionAnalyzer;
pub use metadata::{
    AngularVersion, ChangeDetectionStrategy, ComponentMetadata, DeclarationListEmitMode,
    HostDirectiveMetadata, HostMetadata, LifecycleMetadata, TemplateDependency,
    TemplateDependencyKind, ViewEncapsulation,
};
pub use namespace_registry::NamespaceRegistry;
pub use transform::{
    CompiledComponent, HmrTemplateCompileOutput, HostMetadataInput, ImportInfo, ImportMap,
    LinkerHostBindingOutput, LinkerTemplateOutput, ResolvedResources, ResolvedTypeScriptOptions,
    TemplateCompileOutput, TransformOptions, TransformResult, TypeScriptOption, build_import_map,
    compile_component_template, compile_for_hmr, compile_host_bindings_for_linker,
    compile_template_for_hmr, compile_template_for_linker, compile_template_to_js,
    compile_template_to_js_with_options, transform_angular_file,
};

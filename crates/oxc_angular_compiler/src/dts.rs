//! Angular `.d.ts` type declaration generation.
//!
//! This module generates the static type declarations that should be added
//! to `.d.ts` files for Angular library builds. These declarations enable
//! Angular's template type-checking system to work with pre-compiled libraries.
//!
//! The generated declarations use `i0` as the namespace alias for `@angular/core`,
//! matching Angular's convention. Consumers must ensure their `.d.ts` files include:
//! ```typescript
//! import * as i0 from "@angular/core";
//! ```
//!
//! Reference: Angular's `IvyDeclarationDtsTransform` in
//! `packages/compiler-cli/src/ngtsc/transform/src/declaration.ts`

use crate::component::{ComponentMetadata, HostDirectiveMetadata, R3DependencyMetadata};
use crate::directive::{R3DirectiveMetadata, R3InputMetadata};
use crate::injectable::InjectableMetadata;
use crate::ng_module::NgModuleMetadata;
use crate::pipe::PipeMetadata;

/// A `.d.ts` type declaration for an Angular class.
///
/// Contains the class name and the static member declarations
/// that should be injected into the corresponding `.d.ts` class.
#[derive(Debug, Clone, Default)]
pub struct DtsDeclaration {
    /// The name of the class.
    pub class_name: String,
    /// The static member declarations to add to the class body in `.d.ts`.
    /// This is a newline-separated string of `static` property declarations.
    ///
    /// Example:
    /// ```text
    /// static ɵfac: i0.ɵɵFactoryDeclaration<MyComponent, never>;
    /// static ɵcmp: i0.ɵɵComponentDeclaration<MyComponent, "app-my", never, {}, {}, never, never, true, never>;
    /// ```
    pub members: String,
}

// =============================================================================
// Component Declarations
// =============================================================================

/// Generate `.d.ts` declarations for a `@Component` class.
///
/// Produces:
/// - `static ɵfac: i0.ɵɵFactoryDeclaration<T, CtorDeps>;`
/// - `static ɵcmp: i0.ɵɵComponentDeclaration<T, Selector, ExportAs, InputMap, OutputMap, QueryFields, NgContentSelectors, IsStandalone, HostDirectives, IsSignal>;`
pub fn generate_component_dts(
    metadata: &ComponentMetadata,
    type_argument_count: u32,
    content_query_names: &[String],
    has_injectable: bool,
    ng_content_selectors: &[String],
) -> DtsDeclaration {
    let class_name = metadata.class_name.as_str();
    let type_with_params = type_with_parameters(class_name, type_argument_count);

    // ɵfac declaration
    let ctor_deps_type = generate_ctor_deps_type_from_component_deps(
        metadata.constructor_deps.as_ref().map(|v| v.as_slice() as &[R3DependencyMetadata]),
    );
    let fac =
        format!("static ɵfac: i0.ɵɵFactoryDeclaration<{type_with_params}, {ctor_deps_type}>;");

    // ɵcmp declaration
    let selector = match &metadata.selector {
        Some(s) => {
            // Remove newlines from selector (matching Angular TS behavior)
            let cleaned = s.as_str().replace('\n', "");
            format!("\"{}\"", escape_dts_string(&cleaned))
        }
        None => "never".to_string(),
    };

    let export_as = if metadata.export_as.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            metadata
                .export_as
                .iter()
                .map(|e| format!("\"{}\"", escape_dts_string(e.as_str())))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let input_map = generate_input_map_type(&metadata.inputs);
    let output_map = generate_output_map_type(&metadata.outputs);

    let query_fields = if content_query_names.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            content_query_names
                .iter()
                .map(|name| format!("\"{}\"", escape_dts_string(name)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    // NgContentSelectors: format as tuple type from template ng-content selectors
    let ng_content_selectors = if ng_content_selectors.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            ng_content_selectors
                .iter()
                .map(|s| format!("\"{}\"", escape_dts_string(s)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let is_standalone = if metadata.standalone { "true" } else { "false" };

    let host_directives = if metadata.host_directives.is_empty() {
        "never".to_string()
    } else {
        generate_host_directives_type_from_component(&metadata.host_directives)
    };

    let mut type_params = vec![
        type_with_params.clone(),
        selector,
        export_as,
        input_map,
        output_map,
        query_fields,
        ng_content_selectors,
        is_standalone.to_string(),
        host_directives,
    ];

    if metadata.is_signal {
        type_params.push("true".to_string());
    }

    let cmp = format!("static ɵcmp: i0.ɵɵComponentDeclaration<{}>;", type_params.join(", "));

    let mut members = format!("{fac}\n{cmp}");

    // Add ɵprov if @Injectable is also present
    if has_injectable {
        members
            .push_str(&format!("\nstatic ɵprov: i0.ɵɵInjectableDeclaration<{type_with_params}>;"));
    }

    // Add ngAcceptInputType_* fields for non-signal inputs with transform functions
    generate_input_transform_fields(&metadata.inputs, &mut members);

    DtsDeclaration { class_name: class_name.to_string(), members }
}

// =============================================================================
// Directive Declarations
// =============================================================================

/// Generate `.d.ts` declarations for a `@Directive` class.
///
/// Produces:
/// - `static ɵfac: i0.ɵɵFactoryDeclaration<T, CtorDeps>;`
/// - `static ɵdir: i0.ɵɵDirectiveDeclaration<T, Selector, ExportAs, InputMap, OutputMap, QueryFields, never, IsStandalone, HostDirectives, IsSignal>;`
pub fn generate_directive_dts(
    metadata: &R3DirectiveMetadata,
    has_injectable: bool,
) -> DtsDeclaration {
    let class_name = metadata.name.as_str();
    let type_with_params = type_with_parameters(class_name, metadata.type_argument_count);

    // ɵfac declaration
    let ctor_deps_type =
        generate_ctor_deps_type_from_factory_deps(metadata.deps.as_ref().map(|v| v.as_slice()));
    let fac =
        format!("static ɵfac: i0.ɵɵFactoryDeclaration<{type_with_params}, {ctor_deps_type}>;");

    // ɵdir declaration
    let selector = match &metadata.selector {
        Some(s) => {
            let cleaned = s.as_str().replace('\n', "");
            format!("\"{}\"", escape_dts_string(&cleaned))
        }
        None => "never".to_string(),
    };

    let export_as = if metadata.export_as.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            metadata
                .export_as
                .iter()
                .map(|e| format!("\"{}\"", escape_dts_string(e.as_str())))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let input_map = generate_input_map_type(&metadata.inputs);
    let output_map = generate_output_map_type(&metadata.outputs);

    let query_fields = if metadata.queries.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            metadata
                .queries
                .iter()
                .map(|q| format!("\"{}\"", escape_dts_string(q.property_name.as_str())))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    // NgContentSelectors is always `never` for directives
    let ng_content_selectors = "never";

    let is_standalone = if metadata.is_standalone { "true" } else { "false" };

    let host_directives = if metadata.host_directives.is_empty() {
        "never".to_string()
    } else {
        generate_host_directives_type_from_directive(&metadata.host_directives)
    };

    let mut type_params = vec![
        type_with_params.clone(),
        selector,
        export_as,
        input_map,
        output_map,
        query_fields,
        ng_content_selectors.to_string(),
        is_standalone.to_string(),
        host_directives,
    ];

    if metadata.is_signal {
        type_params.push("true".to_string());
    }

    let dir = format!("static ɵdir: i0.ɵɵDirectiveDeclaration<{}>;", type_params.join(", "));

    let mut members = format!("{fac}\n{dir}");

    if has_injectable {
        members
            .push_str(&format!("\nstatic ɵprov: i0.ɵɵInjectableDeclaration<{type_with_params}>;"));
    }

    // Add ngAcceptInputType_* fields for non-signal inputs with transform functions
    generate_input_transform_fields(&metadata.inputs, &mut members);

    DtsDeclaration { class_name: class_name.to_string(), members }
}

// =============================================================================
// Pipe Declarations
// =============================================================================

/// Generate `.d.ts` declarations for a `@Pipe` class.
///
/// Produces:
/// - `static ɵfac: i0.ɵɵFactoryDeclaration<T, CtorDeps>;`
/// - `static ɵpipe: i0.ɵɵPipeDeclaration<T, Name, IsStandalone>;`
pub fn generate_pipe_dts(
    metadata: &PipeMetadata,
    type_argument_count: u32,
    has_injectable: bool,
) -> DtsDeclaration {
    let class_name = metadata.class_name.as_str();
    let type_with_params = type_with_parameters(class_name, type_argument_count);

    // ɵfac declaration
    let ctor_deps_type =
        generate_ctor_deps_type_from_factory_deps(metadata.deps.as_ref().map(|v| v.as_slice()));
    let fac =
        format!("static ɵfac: i0.ɵɵFactoryDeclaration<{type_with_params}, {ctor_deps_type}>;");

    // ɵpipe declaration
    let pipe_name = match &metadata.pipe_name {
        Some(name) => format!("\"{}\"", escape_dts_string(name.as_str())),
        None => "null".to_string(),
    };

    let is_standalone = if metadata.standalone { "true" } else { "false" };

    let pipe = format!(
        "static ɵpipe: i0.ɵɵPipeDeclaration<{type_with_params}, {pipe_name}, {is_standalone}>;"
    );

    let mut members = format!("{fac}\n{pipe}");

    if has_injectable {
        members
            .push_str(&format!("\nstatic ɵprov: i0.ɵɵInjectableDeclaration<{type_with_params}>;"));
    }

    DtsDeclaration { class_name: class_name.to_string(), members }
}

// =============================================================================
// NgModule Declarations
// =============================================================================

/// Generate `.d.ts` declarations for a `@NgModule` class.
///
/// Produces:
/// - `static ɵfac: i0.ɵɵFactoryDeclaration<T, CtorDeps>;`
/// - `static ɵmod: i0.ɵɵNgModuleDeclaration<T, Declarations, Imports, Exports>;`
/// - `static ɵinj: i0.ɵɵInjectorDeclaration<T>;`
pub fn generate_ng_module_dts(
    metadata: &NgModuleMetadata,
    type_argument_count: u32,
    has_injectable: bool,
) -> DtsDeclaration {
    let class_name = metadata.class_name.as_str();
    let type_with_params = type_with_parameters(class_name, type_argument_count);

    // ɵfac declaration
    let ctor_deps_type =
        generate_ctor_deps_type_from_factory_deps(metadata.deps.as_ref().map(|v| v.as_slice()));
    let fac =
        format!("static ɵfac: i0.ɵɵFactoryDeclaration<{type_with_params}, {ctor_deps_type}>;");

    // ɵmod declaration - uses typeof references for declarations/imports/exports
    let declarations_type = if metadata.declarations.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            metadata
                .declarations
                .iter()
                .map(|d| format!("typeof {}", d.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let imports_type = if metadata.imports.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            metadata
                .imports
                .iter()
                .map(|i| format!("typeof {}", i.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let exports_type = if metadata.exports.is_empty() {
        "never".to_string()
    } else {
        format!(
            "[{}]",
            metadata
                .exports
                .iter()
                .map(|e| format!("typeof {}", e.as_str()))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };

    let mod_decl = format!(
        "static ɵmod: i0.ɵɵNgModuleDeclaration<{type_with_params}, {declarations_type}, {imports_type}, {exports_type}>;"
    );

    // ɵinj declaration
    let inj = format!("static ɵinj: i0.ɵɵInjectorDeclaration<{type_with_params}>;");

    let mut members = format!("{fac}\n{mod_decl}\n{inj}");

    if has_injectable {
        members
            .push_str(&format!("\nstatic ɵprov: i0.ɵɵInjectableDeclaration<{type_with_params}>;"));
    }

    DtsDeclaration { class_name: class_name.to_string(), members }
}

// =============================================================================
// Injectable Declarations
// =============================================================================

/// Generate `.d.ts` declarations for a standalone `@Injectable` class.
///
/// Produces:
/// - `static ɵfac: i0.ɵɵFactoryDeclaration<T, CtorDeps>;`
/// - `static ɵprov: i0.ɵɵInjectableDeclaration<T>;`
pub fn generate_injectable_dts(
    metadata: &InjectableMetadata,
    type_argument_count: u32,
) -> DtsDeclaration {
    let class_name = metadata.class_name.as_str();
    let type_with_params = type_with_parameters(class_name, type_argument_count);

    // ɵfac declaration
    let ctor_deps_type =
        generate_ctor_deps_type_from_factory_deps(metadata.deps.as_ref().map(|v| v.as_slice()));
    let fac =
        format!("static ɵfac: i0.ɵɵFactoryDeclaration<{type_with_params}, {ctor_deps_type}>;");

    // ɵprov declaration
    let prov = format!("static ɵprov: i0.ɵɵInjectableDeclaration<{type_with_params}>;");

    let members = format!("{fac}\n{prov}");

    DtsDeclaration { class_name: class_name.to_string(), members }
}

// =============================================================================
// Helper Functions
// =============================================================================

/// Generate the type parameter `T` with any generic params filled as `any`.
///
/// For `class Foo<A, B>`, produces `Foo<any, any>`.
/// For `class Foo`, produces `Foo`.
fn type_with_parameters(class_name: &str, count: u32) -> String {
    if count == 0 {
        class_name.to_string()
    } else {
        let params: Vec<&str> = (0..count).map(|_| "any").collect();
        format!("{}<{}>", class_name, params.join(", "))
    }
}

/// Shared helper: given an iterator of per-dependency info tuples, produce the
/// constructor deps type string (`never` or a tuple type like
/// `[null, {attribute: "title", optional: true}, null]`).
///
/// Each tuple element is `(attribute_entry, optional, host, self_, skip_self)`.
/// `attribute_entry` is the pre-formatted `attribute: …` string fragment when
/// the dependency has an attribute flag (e.g. `attribute: "title"` or
/// `attribute: string`), or `None` otherwise.
fn generate_ctor_deps_type(
    deps: impl Iterator<Item = (Option<String>, bool, bool, bool, bool)>,
) -> String {
    let dep_types: Vec<Option<String>> = deps
        .map(|(attribute_entry, optional, host, self_, skip_self)| {
            let mut entries: Vec<String> = Vec::new();
            if let Some(attr) = attribute_entry {
                entries.push(attr);
            }
            if optional {
                entries.push("optional: true".to_string());
            }
            if host {
                entries.push("host: true".to_string());
            }
            if self_ {
                entries.push("self: true".to_string());
            }
            if skip_self {
                entries.push("skipSelf: true".to_string());
            }
            if entries.is_empty() { None } else { Some(format!("{{ {} }}", entries.join(", "))) }
        })
        .collect();

    let has_types = dep_types.iter().any(|t| t.is_some());
    if !has_types {
        "never".to_string()
    } else {
        let entries: Vec<String> =
            dep_types.into_iter().map(|t| t.unwrap_or_else(|| "null".to_string())).collect();
        format!("[{}]", entries.join(", "))
    }
}

/// Generate the constructor deps type parameter for `ɵɵFactoryDeclaration`.
///
/// Returns `never` if no dependency has any special flags (attribute, optional, host, self, skipSelf).
/// Otherwise returns a tuple type like `[null, {attribute: "title", optional: true}, null]`.
fn generate_ctor_deps_type_from_component_deps(deps: Option<&[R3DependencyMetadata]>) -> String {
    match deps {
        None => "never".to_string(),
        Some(deps) => generate_ctor_deps_type(deps.iter().map(|d| {
            let attribute_entry = d
                .attribute_name
                .as_ref()
                .map(|name| format!("attribute: \"{}\"", escape_dts_string(name.as_str())));
            (attribute_entry, d.optional, d.host, d.self_, d.skip_self)
        })),
    }
}

/// Generate the constructor deps type from directive/pipe/ngmodule/injectable deps.
///
/// Uses the factory module's `R3DependencyMetadata` which is used by directives,
/// pipes, NgModules, and injectables.
///
/// Returns `never` if no dependency has any special flags (attribute, optional, host, self, skipSelf).
/// Otherwise returns a tuple type like `[null, {attribute: string, optional: true}, null]`.
fn generate_ctor_deps_type_from_factory_deps(
    deps: Option<&[crate::factory::R3DependencyMetadata]>,
) -> String {
    match deps {
        None => "never".to_string(),
        Some(deps) => generate_ctor_deps_type(deps.iter().map(|d| {
            let attribute_entry = if d.attribute_name_type.is_some() {
                Some("attribute: string".to_string())
            } else {
                None
            };
            (attribute_entry, d.optional, d.host, d.self_, d.skip_self)
        })),
    }
}

/// Generate `ngAcceptInputType_*` static fields for non-signal inputs with transform functions.
///
/// When an input has a `transform` function (e.g., `@Input({transform: booleanAttribute})`),
/// Angular generates a static field like:
/// ```text
/// static ngAcceptInputType_disabled: unknown;
/// ```
/// This enables template type-checking to know that transformed inputs accept wider types.
///
/// Signal inputs do NOT generate these fields (they capture WriteT within the InputSignal type).
///
/// Note: We use `unknown` as the type because we don't have access to the TypeScript type checker
/// to determine the actual write type of the transform function.
fn generate_input_transform_fields(inputs: &[R3InputMetadata], members: &mut String) {
    for input in inputs {
        if !input.is_signal && input.transform_function.is_some() {
            members.push_str(&format!(
                "\nstatic ngAcceptInputType_{}: unknown;",
                input.class_property_name.as_str()
            ));
        }
    }
}

/// Generate the input map type for `ɵɵComponentDeclaration` / `ɵɵDirectiveDeclaration`.
///
/// Produces a TypeScript object literal type like:
/// ```text
/// { "name": { "alias": "name"; "required": false; }; "value": { "alias": "aliasedValue"; "required": true; "isSignal": true; }; }
/// ```
fn generate_input_map_type(inputs: &[R3InputMetadata]) -> String {
    if inputs.is_empty() {
        return "{}".to_string();
    }

    let entries: Vec<String> = inputs
        .iter()
        .map(|input| {
            let key = escape_dts_string(input.class_property_name.as_str());
            let alias = escape_dts_string(input.binding_property_name.as_str());
            let required = if input.required { "true" } else { "false" };

            let mut props = format!("\"alias\": \"{alias}\"; \"required\": {required};");
            if input.is_signal {
                props.push_str(" \"isSignal\": true;");
            }

            format!("\"{key}\": {{ {props} }};")
        })
        .collect();

    format!("{{ {} }}", entries.join(" "))
}

/// Generate the output map type.
///
/// Produces: `{ "clicked": "clicked"; "valueChanged": "onChange"; }`
fn generate_output_map_type(outputs: &[(oxc_span::Atom, oxc_span::Atom)]) -> String {
    if outputs.is_empty() {
        return "{}".to_string();
    }

    let entries: Vec<String> = outputs
        .iter()
        .map(|(class_name, binding_name)| {
            format!(
                "\"{}\": \"{}\";",
                escape_dts_string(class_name.as_str()),
                escape_dts_string(binding_name.as_str())
            )
        })
        .collect();

    format!("{{ {} }}", entries.join(" "))
}

/// Generate the host directives type from component host directives.
fn generate_host_directives_type_from_component(
    host_directives: &[HostDirectiveMetadata],
) -> String {
    let entries: Vec<String> = host_directives
        .iter()
        .map(|hd| {
            let directive = format!("typeof {}", hd.directive.as_str());
            let inputs = if hd.inputs.is_empty() {
                "{}".to_string()
            } else {
                let input_entries: Vec<String> = hd
                    .inputs
                    .iter()
                    .map(|(public, internal)| {
                        format!(
                            "\"{}\": \"{}\"",
                            escape_dts_string(public.as_str()),
                            escape_dts_string(internal.as_str())
                        )
                    })
                    .collect();
                format!("{{ {} }}", input_entries.join("; "))
            };
            let outputs = if hd.outputs.is_empty() {
                "{}".to_string()
            } else {
                let output_entries: Vec<String> = hd
                    .outputs
                    .iter()
                    .map(|(public, internal)| {
                        format!(
                            "\"{}\": \"{}\"",
                            escape_dts_string(public.as_str()),
                            escape_dts_string(internal.as_str())
                        )
                    })
                    .collect();
                format!("{{ {} }}", output_entries.join("; "))
            };
            format!("{{ directive: {directive}; inputs: {inputs}; outputs: {outputs}; }}")
        })
        .collect();

    format!("[{}]", entries.join(", "))
}

/// Generate the host directives type from directive host directives.
fn generate_host_directives_type_from_directive(
    host_directives: &[crate::directive::R3HostDirectiveMetadata],
) -> String {
    let entries: Vec<String> = host_directives
        .iter()
        .map(|hd| {
            // Extract the directive name from the OutputExpression
            let directive_name = extract_directive_name_from_expr(&hd.directive);
            let directive = format!("typeof {directive_name}");
            let inputs = if hd.inputs.is_empty() {
                "{}".to_string()
            } else {
                let input_entries: Vec<String> = hd
                    .inputs
                    .iter()
                    .map(|(public, internal)| {
                        format!(
                            "\"{}\": \"{}\"",
                            escape_dts_string(public.as_str()),
                            escape_dts_string(internal.as_str())
                        )
                    })
                    .collect();
                format!("{{ {} }}", input_entries.join("; "))
            };
            let outputs = if hd.outputs.is_empty() {
                "{}".to_string()
            } else {
                let output_entries: Vec<String> = hd
                    .outputs
                    .iter()
                    .map(|(public, internal)| {
                        format!(
                            "\"{}\": \"{}\"",
                            escape_dts_string(public.as_str()),
                            escape_dts_string(internal.as_str())
                        )
                    })
                    .collect();
                format!("{{ {} }}", output_entries.join("; "))
            };
            format!("{{ directive: {directive}; inputs: {inputs}; outputs: {outputs}; }}")
        })
        .collect();

    format!("[{}]", entries.join(", "))
}

/// Extract a directive name from an `OutputExpression`.
///
/// Handles:
/// - `ReadVar`: simple variable name (e.g. `SomeDirective`)
/// - `ReadProp`: namespace-qualified reference (e.g. `i1.SomeDirective`)
/// - `External`: external module reference, using the export name
fn extract_directive_name_from_expr(expr: &crate::output::ast::OutputExpression) -> String {
    match expr {
        crate::output::ast::OutputExpression::ReadVar(read_var) => {
            read_var.name.as_str().to_string()
        }
        crate::output::ast::OutputExpression::ReadProp(read_prop) => {
            let receiver = extract_directive_name_from_expr(&read_prop.receiver);
            format!("{}.{}", receiver, read_prop.name.as_str())
        }
        crate::output::ast::OutputExpression::External(external) => match &external.value.name {
            Some(name) => name.as_str().to_string(),
            None => panic!("ExternalExpr in host directive has no export name"),
        },
        other => {
            panic!(
                "Unexpected OutputExpression variant in host directive type: {:?}",
                std::mem::discriminant(other)
            )
        }
    }
}

/// Escape a string for use in a TypeScript `.d.ts` string literal type.
fn escape_dts_string(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_type_with_parameters_no_params() {
        assert_eq!(type_with_parameters("MyComponent", 0), "MyComponent");
    }

    #[test]
    fn test_type_with_parameters_with_params() {
        assert_eq!(type_with_parameters("MyComponent", 2), "MyComponent<any, any>");
    }

    #[test]
    fn test_escape_dts_string() {
        assert_eq!(escape_dts_string("hello"), "hello");
        assert_eq!(escape_dts_string(r#"he"llo"#), r#"he\"llo"#);
        assert_eq!(escape_dts_string(r"he\llo"), r"he\\llo");
        assert_eq!(escape_dts_string("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_dts_string("col1\tcol2"), "col1\\tcol2");
        assert_eq!(escape_dts_string("a\r\nb"), "a\\r\\nb");
    }

    #[test]
    fn test_generate_input_map_type_empty() {
        let inputs: Vec<R3InputMetadata> = vec![];
        assert_eq!(generate_input_map_type(&inputs), "{}");
    }

    #[test]
    fn test_generate_output_map_type_empty() {
        let outputs: Vec<(oxc_span::Atom, oxc_span::Atom)> = vec![];
        assert_eq!(generate_output_map_type(&outputs), "{}");
    }
}

//! Directive compilation implementation.
//!
//! Ported from Angular's `render3/view/compiler.ts`.
//!
//! Generates directive definitions like:
//! ```javascript
//! ɵdir = ɵɵdefineDirective({
//!   type: MyDirective,
//!   selectors: [['', 'myDir', '']],
//!   inputs: { prop: 'prop' },
//!   outputs: { click: 'click' },
//!   hostBindings: function(rf, ctx) { ... },
//!   features: [ɵɵNgOnChangesFeature]
//! })
//! ```

use oxc_allocator::{Allocator, Box, Vec};
use oxc_span::{Atom, Span};
use rustc_hash::FxHashMap;

use super::metadata::{
    R3DirectiveMetadata, R3HostDirectiveMetadata, R3HostMetadata, R3InputMetadata,
};
use crate::ast::expression::{BindingType, ParsedEventType};
use crate::ast::r3::{R3BoundAttribute, R3BoundEvent, SecurityContext};
use crate::output::ast::{
    FunctionExpr, InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr, LiteralMapEntry,
    LiteralMapExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
};
use crate::parser::expression::BindingParser;
use crate::pipeline::emit::{HostBindingCompilationResult, compile_host_bindings};
use crate::pipeline::ingest::{HostBindingInput, ingest_host_binding};
use crate::pipeline::selector::{
    parse_selector_to_r3_selector as parse_css_to_r3, r3_selector_to_output_expr,
};
use crate::r3::Identifiers;

/// Result of compiling a directive.
#[derive(Debug)]
pub struct DirectiveCompileResult<'a> {
    /// The compiled expression: `ɵɵdefineDirective({...})`
    pub expression: OutputExpression<'a>,

    /// Additional statements (usually empty).
    pub statements: Vec<'a, OutputStatement<'a>>,

    /// The next available pool index after compilation.
    /// Used to track constant pool usage across multiple directives in the same file.
    pub next_pool_index: u32,
}

/// Compiles a directive from its metadata.
///
/// This is the main entry point for directive compilation.
///
/// The `pool_starting_index` parameter is used to ensure constant names don't conflict
/// when compiling multiple directives in the same file. Each directive continues from
/// where the previous directive's pool left off.
pub fn compile_directive<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    pool_starting_index: u32,
) -> DirectiveCompileResult<'a> {
    compile_directive_from_metadata(allocator, metadata, pool_starting_index)
}

/// Internal implementation of directive compilation.
pub fn compile_directive_from_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    pool_starting_index: u32,
) -> DirectiveCompileResult<'a> {
    // Build the base directive fields, passing pool_starting_index for host bindings
    let (definition_map, next_pool_index, host_declarations) =
        build_base_directive_fields(allocator, metadata, pool_starting_index);

    // Add features
    let mut definition_map = definition_map;
    add_features(allocator, metadata, &mut definition_map);

    // Create the expression: ɵɵdefineDirective(definitionMap)
    let expression = create_define_directive_call(allocator, definition_map);

    // Convert host binding declarations to statements
    let mut statements = Vec::new_in(allocator);
    for decl in host_declarations {
        statements.push(decl);
    }

    DirectiveCompileResult { expression, statements, next_pool_index }
}

/// Builds the base directive definition map.
///
/// Corresponds to `baseDirectiveFields()` in Angular's compiler.
///
/// Returns a tuple of (entries, next_pool_index, host_declarations) where next_pool_index is the
/// next available constant pool index after host binding compilation, and host_declarations
/// contains any pooled constants (pure functions) from host binding compilation.
fn build_base_directive_fields<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    pool_starting_index: u32,
) -> (Vec<'a, LiteralMapEntry<'a>>, u32, oxc_allocator::Vec<'a, OutputStatement<'a>>) {
    let mut entries = Vec::new_in(allocator);
    let mut next_pool_index = pool_starting_index;
    let mut host_declarations = oxc_allocator::Vec::new_in(allocator);

    // type: MyDirective
    entries.push(LiteralMapEntry {
        key: Atom::from("type"),
        value: metadata.r#type.clone_in(allocator),
        quoted: false,
    });

    // selectors: [['', 'myDir', '']]
    if let Some(selector) = &metadata.selector {
        if let Some(selectors_expr) = parse_selector_to_r3_selector(allocator, selector) {
            entries.push(LiteralMapEntry {
                key: Atom::from("selectors"),
                value: selectors_expr,
                quoted: false,
            });
        }
    }

    // contentQueries: (rf, ctx, dirIndex) => { ... }
    if !metadata.queries.is_empty() {
        // Note: Directive compiler doesn't have access to constant pool, so predicates
        // are not pooled. For components, pool is passed from component compilation.
        let content_queries_fn = super::query::create_content_queries_function(
            allocator,
            &metadata.queries,
            Some(metadata.name.as_str()),
            None,
        );
        entries.push(LiteralMapEntry {
            key: Atom::from("contentQueries"),
            value: content_queries_fn,
            quoted: false,
        });
    }

    // viewQuery: (rf, ctx) => { ... }
    if !metadata.view_queries.is_empty() {
        // Note: Directive compiler doesn't have access to constant pool, so predicates
        // are not pooled. For components, pool is passed from component compilation.
        let view_queries_fn = super::query::create_view_queries_function(
            allocator,
            &metadata.view_queries,
            Some(metadata.name.as_str()),
            None,
        );
        entries.push(LiteralMapEntry {
            key: Atom::from("viewQuery"),
            value: view_queries_fn,
            quoted: false,
        });
    }

    // hostBindings: (rf, ctx) => { ... }
    // Uses the IR pipeline for proper host binding compilation
    // Per Angular's compiler.ts lines 525-532, createHostBindingsFunction sets:
    // - hostAttrs: static host attributes (only Attribute, ClassName, StyleProperty)
    // - hostVars: number of host variables (only if > 0)
    // - hostBindings: the host binding function
    if metadata.host.has_bindings() {
        if let Some((result, new_pool_index)) =
            compile_directive_host_bindings(allocator, metadata, pool_starting_index)
        {
            next_pool_index = new_pool_index;

            // hostAttrs: [...] - static host attributes
            // Note: Property/TwoWayProperty bindings are excluded from hostAttrs
            // as they are dynamic bindings handled by hostBindings function
            if let Some(host_attrs) = result.host_attrs {
                entries.push(LiteralMapEntry {
                    key: Atom::from("hostAttrs"),
                    value: host_attrs,
                    quoted: false,
                });
            }

            // hostVars: number - only if > 0
            if let Some(host_vars) = result.host_vars {
                entries.push(LiteralMapEntry {
                    key: Atom::from("hostVars"),
                    value: OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::Number(host_vars as f64),
                            source_span: None,
                        },
                        allocator,
                    )),
                    quoted: false,
                });
            }

            // hostBindings: function(rf, ctx) { ... }
            if let Some(host_fn) = result.host_binding_fn {
                entries.push(LiteralMapEntry {
                    key: Atom::from("hostBindings"),
                    value: OutputExpression::Function(Box::new_in(host_fn, allocator)),
                    quoted: false,
                });
            }

            // Collect host binding pool declarations (pure functions, etc.)
            host_declarations = result.declarations;
        }
    }

    // inputs: { prop: 'prop', aliased: ['publicName', 'privateField'] }
    if !metadata.inputs.is_empty() {
        if let Some(inputs_expr) = create_inputs_literal(allocator, &metadata.inputs) {
            entries.push(LiteralMapEntry {
                key: Atom::from("inputs"),
                value: inputs_expr,
                quoted: false,
            });
        }
    }

    // outputs: { click: 'click' }
    if !metadata.outputs.is_empty() {
        if let Some(outputs_expr) = create_outputs_literal(allocator, &metadata.outputs) {
            entries.push(LiteralMapEntry {
                key: Atom::from("outputs"),
                value: outputs_expr,
                quoted: false,
            });
        }
    }

    // exportAs: ['myDir']
    if !metadata.export_as.is_empty() {
        let mut export_items = Vec::new_in(allocator);
        for name in &metadata.export_as {
            export_items.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
                allocator,
            )));
        }
        entries.push(LiteralMapEntry {
            key: Atom::from("exportAs"),
            value: OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: export_items, source_span: None },
                allocator,
            )),
            quoted: false,
        });
    }

    // standalone: false (only if not standalone, since true is default)
    if !metadata.is_standalone {
        entries.push(LiteralMapEntry {
            key: Atom::from("standalone"),
            value: OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(false), source_span: None },
                allocator,
            )),
            quoted: false,
        });
    }

    // signals: true (only if signal-based)
    if metadata.is_signal {
        entries.push(LiteralMapEntry {
            key: Atom::from("signals"),
            value: OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(true), source_span: None },
                allocator,
            )),
            quoted: false,
        });
    }

    (entries, next_pool_index, host_declarations)
}

/// Adds features to the definition map.
///
/// Corresponds to `addFeatures()` in Angular's compiler.
fn add_features<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    definition_map: &mut Vec<'a, LiteralMapEntry<'a>>,
) {
    let mut features = Vec::new_in(allocator);

    // ProvidersFeature
    if let Some(providers) = &metadata.providers {
        let mut args = Vec::new_in(allocator);
        args.push(providers.clone_in(allocator));
        features.push(create_feature_call(allocator, Identifiers::PROVIDERS_FEATURE, args));
    }

    // HostDirectivesFeature (before InheritDefinitionFeature)
    if !metadata.host_directives.is_empty() {
        let host_directives_arg =
            create_host_directives_feature_arg(allocator, &metadata.host_directives);
        let mut args = Vec::new_in(allocator);
        args.push(host_directives_arg);
        features.push(create_feature_call(allocator, Identifiers::HOST_DIRECTIVES_FEATURE, args));
    }

    // InheritDefinitionFeature
    if metadata.uses_inheritance {
        features.push(create_feature_ref(allocator, Identifiers::INHERIT_DEFINITION_FEATURE));
    }

    // NgOnChangesFeature
    if metadata.uses_on_changes {
        features.push(create_feature_ref(allocator, Identifiers::NG_ON_CHANGES_FEATURE));
    }

    if !features.is_empty() {
        definition_map.push(LiteralMapEntry {
            key: Atom::from("features"),
            value: OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: features, source_span: None },
                allocator,
            )),
            quoted: false,
        });
    }
}

/// Creates the `ɵɵdefineDirective({...})` call expression.
fn create_define_directive_call<'a>(
    allocator: &'a Allocator,
    definition_map: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    // Create i0.ɵɵdefineDirective
    let define_directive_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Atom::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Atom::from(Identifiers::DEFINE_DIRECTIVE),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create the literal map expression
    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: definition_map, source_span: None },
        allocator,
    ));

    // Create the function call
    let mut args = Vec::new_in(allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(define_directive_fn, allocator),
            args,
            pure: true,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Parses a CSS selector string to R3 selector format.
///
/// Uses the full CSS selector parser to correctly handle compound selectors like:
/// - `ng-template[body]` -> `[["ng-template", "body", ""]]`
/// - `span[bitBadge]` -> `[["span", "bitBadge", ""]]`
/// - `[myDir]` -> `[["", "myDir", ""]]`
/// - `.my-class` -> `[["", 8, "my-class"]]` (8 = CLASS flag)
/// - `button[type="submit"]` -> `[["button", "type", "submit"]]`
///
/// Ported from Angular's `parseSelectorToR3Selector` in `core.ts`.
fn parse_selector_to_r3_selector<'a>(
    allocator: &'a Allocator,
    selector: &Atom<'a>,
) -> Option<OutputExpression<'a>> {
    let selector_str = selector.as_str();
    if selector_str.is_empty() {
        return None;
    }

    // Use the proper CSS selector parser from pipeline/selector.rs
    let r3_selectors = parse_css_to_r3(selector_str);

    if r3_selectors.is_empty() {
        return None;
    }

    // Convert each R3 selector to an output expression array
    let mut outer_array = Vec::new_in(allocator);

    for r3_selector in &r3_selectors {
        let inner_entries = r3_selector_to_output_expr(allocator, r3_selector);
        outer_array.push(OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries: inner_entries, source_span: None },
            allocator,
        )));
    }

    Some(OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: outer_array, source_span: None },
        allocator,
    )))
}

/// Input flags for directive inputs.
///
/// Corresponds to Angular's `InputFlags` enum in `core.ts`.
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum InputFlags {
    None = 0,
    SignalBased = 1,                // 1 << 0
    HasDecoratorInputTransform = 2, // 1 << 1
}

/// Creates the inputs literal map.
///
/// Ported from Angular's `conditionallyCreateDirectiveBindingLiteral` in `render3/view/util.ts`.
///
/// Generates optimized data structures to minimize memory or file size:
/// - Simple case: `{ prop: "prop" }` when class property equals binding property, no transform, and not signal
/// - Complex case: `{ prop: [flags, "publicName", "declaredName"?, transformFunction?] }`
///
/// The format for complex inputs is:
/// - Signal inputs: `[1, "publicName"]` or `[1, "publicName", "declaredName"]` (flags = 1)
/// - Decorator inputs with transform: `[2, "publicName", "declaredName", transformFn]` (flags = 2)
/// - Decorator inputs with alias only: `[0, "publicName", "declaredName"]` (flags = 0)
pub fn create_inputs_literal<'a>(
    allocator: &'a Allocator,
    inputs: &[R3InputMetadata<'a>],
) -> Option<OutputExpression<'a>> {
    if inputs.is_empty() {
        return None;
    }

    let mut entries = Vec::new_in(allocator);

    for input in inputs {
        let public_name = &input.binding_property_name;
        let declared_name = &input.class_property_name;
        let different_declaring_name = public_name != declared_name;
        let has_decorator_input_transform = input.transform_function.is_some();

        // Build up input flags
        let mut flags: u8 = InputFlags::None as u8;
        if input.is_signal {
            flags |= InputFlags::SignalBased as u8;
        }
        if has_decorator_input_transform {
            flags |= InputFlags::HasDecoratorInputTransform as u8;
        }

        // Determine if we need the complex array format
        // We need array format if:
        // - Different declaring name (alias)
        // - Has transform function
        // - Has any flags (signal or transform)
        let needs_array = different_declaring_name || has_decorator_input_transform || flags != 0;

        let value = if needs_array {
            // Complex case: create array [flags, publicName, declaredName?, transformFunction?]
            let mut arr_entries: Vec<'a, OutputExpression<'a>> = Vec::new_in(allocator);

            // First element: flags
            arr_entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Number(f64::from(flags)), source_span: None },
                allocator,
            )));

            // Second element: publicName (binding property name)
            arr_entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(public_name.clone()), source_span: None },
                allocator,
            )));

            // Third element: declaredName (class property name) - only if different or has transform
            if different_declaring_name || has_decorator_input_transform {
                arr_entries.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(declared_name.clone()),
                        source_span: None,
                    },
                    allocator,
                )));

                // Fourth element: transformFunction (only if present)
                if let Some(transform) = &input.transform_function {
                    arr_entries.push(transform.clone_in(allocator));
                }
            }

            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: arr_entries, source_span: None },
                allocator,
            ))
        } else {
            // Simple case: just the property name as a string
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(public_name.clone()), source_span: None },
                allocator,
            ))
        };

        entries.push(LiteralMapEntry { key: declared_name.clone(), value, quoted: false });
    }

    Some(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    )))
}

/// Creates the outputs literal map.
pub fn create_outputs_literal<'a>(
    allocator: &'a Allocator,
    outputs: &[(Atom<'a>, Atom<'a>)],
) -> Option<OutputExpression<'a>> {
    if outputs.is_empty() {
        return None;
    }

    let mut entries = Vec::new_in(allocator);

    for (class_name, binding_name) in outputs {
        entries.push(LiteralMapEntry {
            key: class_name.clone(),
            value: OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::String(binding_name.clone()),
                    source_span: None,
                },
                allocator,
            )),
            quoted: false,
        });
    }

    Some(OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    )))
}

/// Compiles host bindings for a directive.
///
/// Ported from Angular's `createHostBindingsFunction` in `compiler.ts`.
/// Uses the IR pipeline for proper host binding compilation.
///
/// The `pool_starting_index` parameter is used to ensure constant names don't conflict
/// when compiling multiple directives in the same file. Each directive continues from
/// where the previous directive's pool left off.
///
/// Returns a tuple of (result, next_pool_index) where next_pool_index is the
/// next available constant pool index after host binding compilation.
fn compile_directive_host_bindings<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    pool_starting_index: u32,
) -> Option<(HostBindingCompilationResult<'a>, u32)> {
    let host = &metadata.host;

    // Check if there are any host bindings at all
    if !host.has_bindings() {
        return None;
    }

    // Get directive name and selector
    let directive_name = metadata.name.clone();
    let directive_selector = metadata.selector.clone().unwrap_or_else(|| Atom::from(""));

    // Convert R3HostMetadata to HostBindingInput
    let input =
        convert_r3_host_metadata_to_input(allocator, host, directive_name, directive_selector);

    // Ingest and compile the host bindings using the IR pipeline
    // Use the provided pool_starting_index to continue from where previous compilations left off
    let mut job = ingest_host_binding(allocator, input, pool_starting_index);
    let result = compile_host_bindings(&mut job);

    // Get the next pool index after host binding compilation
    let next_pool_index = job.pool.next_name_index();

    Some((result, next_pool_index))
}

/// Convert R3HostMetadata to HostBindingInput.
///
/// R3HostMetadata has:
/// - `attributes`: Vec<(Atom, OutputExpression)> - already compiled expressions
/// - `properties`: Vec<(Atom, Atom)> - unparsed property binding strings
/// - `listeners`: Vec<(Atom, Atom)> - unparsed event handler strings
///
/// This function parses the property and listener strings and passes through
/// the already-compiled attribute expressions.
fn convert_r3_host_metadata_to_input<'a>(
    allocator: &'a Allocator,
    host: &R3HostMetadata<'a>,
    directive_name: Atom<'a>,
    directive_selector: Atom<'a>,
) -> HostBindingInput<'a> {
    use oxc_allocator::FromIn;

    let binding_parser = BindingParser::new(allocator);
    let empty_span = Span::empty(0);

    // Convert property bindings: "[class.active]" -> R3BoundAttribute
    let mut properties: Vec<'a, R3BoundAttribute<'a>> = Vec::new_in(allocator);

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
            name: Atom::from_in(final_name, allocator),
            binding_type,
            security_context: SecurityContext::None,
            value: parse_result.ast,
            unit: unit.map(|u| Atom::from_in(u, allocator)),
            source_span: empty_span,
            key_span: empty_span,
            value_span: Some(empty_span),
            i18n: None,
        });
    }

    // Convert event listeners: "(click)" -> R3BoundEvent
    let mut events: Vec<'a, R3BoundEvent<'a>> = Vec::new_in(allocator);

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

        // Parse the handler expression
        let value_str = allocator.alloc_str(value.as_str());
        let parse_result = binding_parser.parse_event(value_str, empty_span);

        events.push(R3BoundEvent {
            name: Atom::from_in(final_event_name, allocator),
            event_type: ParsedEventType::Regular,
            handler: parse_result.ast,
            target: target.map(|t| Atom::from_in(t, allocator)),
            phase: None,
            source_span: empty_span,
            handler_span: empty_span,
            key_span: empty_span,
        });
    }

    // Copy attributes directly - they are already OutputExpressions
    // Handle special style_attr and class_attr if present
    let mut attributes: FxHashMap<Atom<'a>, OutputExpression<'a>> = FxHashMap::default();

    for (key, value) in host.attributes.iter() {
        // Use clone_in to deep clone the OutputExpression with the allocator
        attributes.insert(key.clone(), value.clone_in(allocator));
    }

    // Add special attributes if present
    if let Some(ref style_attr) = host.style_attr {
        let expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(style_attr.clone()), source_span: None },
            allocator,
        ));
        attributes.insert(Atom::from("style"), expr);
    }

    if let Some(ref class_attr) = host.class_attr {
        let expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(class_attr.clone()), source_span: None },
            allocator,
        ));
        attributes.insert(Atom::from("class"), expr);
    }

    HostBindingInput {
        component_name: directive_name,
        component_selector: directive_selector,
        properties,
        attributes,
        events,
    }
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
    } else {
        (BindingType::Property, name, None)
    }
}

/// Parse an event name to extract target prefix (window:, document:, body:).
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

/// Creates a feature call expression: i0.FeatureName(args)
fn create_feature_call<'a>(
    allocator: &'a Allocator,
    feature_name: &'static str,
    args: Vec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    let feature_ref = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Atom::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Atom::from(feature_name),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(feature_ref, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates a feature reference expression: i0.FeatureName
fn create_feature_ref<'a>(
    allocator: &'a Allocator,
    feature_name: &'static str,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Atom::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Atom::from(feature_name),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Creates the host directives feature argument.
fn create_host_directives_feature_arg<'a>(
    allocator: &'a Allocator,
    host_directives: &[R3HostDirectiveMetadata<'a>],
) -> OutputExpression<'a> {
    let mut items = Vec::new_in(allocator);

    for hd in host_directives {
        let mut entries = Vec::new_in(allocator);

        // directive
        let directive_expr = if hd.is_forward_reference {
            // Wrap in forwardRef()
            let mut args = Vec::new_in(allocator);

            let fn_params = Vec::new_in(allocator);
            let mut fn_body = Vec::new_in(allocator);
            fn_body.push(OutputStatement::Return(Box::new_in(
                crate::output::ast::ReturnStatement {
                    value: hd.directive.clone_in(allocator),
                    source_span: None,
                },
                allocator,
            )));

            let arrow_fn = OutputExpression::Function(Box::new_in(
                FunctionExpr {
                    name: None,
                    params: fn_params,
                    statements: fn_body,
                    source_span: None,
                },
                allocator,
            ));

            args.push(arrow_fn);

            let forward_ref = OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Atom::from("i0"), source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    name: Atom::from(Identifiers::FORWARD_REF),
                    optional: false,
                    source_span: None,
                },
                allocator,
            ));

            OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(forward_ref, allocator),
                    args,
                    pure: false,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        } else {
            hd.directive.clone_in(allocator)
        };

        entries.push(LiteralMapEntry {
            key: Atom::from("directive"),
            value: directive_expr,
            quoted: false,
        });

        // inputs (if any)
        if !hd.inputs.is_empty() {
            let inputs_array = create_host_directive_mappings_array(allocator, &hd.inputs);
            entries.push(LiteralMapEntry {
                key: Atom::from("inputs"),
                value: inputs_array,
                quoted: false,
            });
        }

        // outputs (if any)
        if !hd.outputs.is_empty() {
            let outputs_array = create_host_directive_mappings_array(allocator, &hd.outputs);
            entries.push(LiteralMapEntry {
                key: Atom::from("outputs"),
                value: outputs_array,
                quoted: false,
            });
        }

        items.push(OutputExpression::LiteralMap(Box::new_in(
            LiteralMapExpr { entries, source_span: None },
            allocator,
        )));
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: items, source_span: None },
        allocator,
    ))
}

/// Creates a host directive mappings array.
///
/// Format: `['internalName', 'publicName', 'internalName2', 'publicName2']`
///
/// Shared between directive and component compilers, mirroring Angular's
/// `createHostDirectivesMappingArray` in `view/compiler.ts`.
pub(crate) fn create_host_directive_mappings_array<'a>(
    allocator: &'a Allocator,
    mappings: &[(Atom<'a>, Atom<'a>)],
) -> OutputExpression<'a> {
    let mut entries = Vec::with_capacity_in(mappings.len() * 2, allocator);

    for (public_name, internal_name) in mappings {
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(internal_name.clone()), source_span: None },
            allocator,
        )));
        entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(public_name.clone()), source_span: None },
            allocator,
        )));
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directive::metadata::R3HostMetadata;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_compile_simple_directive() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("MyDirective"), source_span: None },
            &allocator,
        ));

        let metadata = R3DirectiveMetadata {
            name: Atom::from("MyDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("[myDir]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let result = compile_directive(&allocator, &metadata, 0);

        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("defineDirective"));
        assert!(output.contains("MyDirective"));
        assert!(output.contains("selectors"));
    }

    #[test]
    fn test_compile_directive_with_inputs_outputs() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("TestDirective"), source_span: None },
            &allocator,
        ));

        let mut inputs = Vec::new_in(&allocator);
        inputs.push(R3InputMetadata::simple(Atom::from("myInput")));

        let mut outputs = Vec::new_in(&allocator);
        outputs.push((Atom::from("myOutput"), Atom::from("myOutput")));

        let metadata = R3DirectiveMetadata {
            name: Atom::from("TestDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("[test]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs,
            outputs,
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("inputs"));
        assert!(output.contains("myInput"));
        assert!(output.contains("outputs"));
        assert!(output.contains("myOutput"));
    }

    #[test]
    fn test_inputs_simple_format() {
        // Test: Simple input (same name, no transform) -> just string
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);
        inputs.push(R3InputMetadata::simple(Atom::from("value")));

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Expected: {value:"value"} - simple string format
        assert!(output.contains(r#"value:"value""#), "Simple input should be string: {}", output);
        // Should NOT contain array brackets for simple case
        assert!(!output.contains("["), "Simple input should not be array: {}", output);
    }

    #[test]
    fn test_inputs_aliased_format() {
        // Test: Aliased input (different publicName vs declaredName) -> array format with flags
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("count"),
            binding_property_name: Atom::from("itemCount"),
            required: false,
            is_signal: false,
            transform_function: None,
        });

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Expected: {count:[0,"itemCount","count"]} - array format with flags=0
        // Key format: [flags, publicName, declaredName]
        assert!(
            output.contains(r#"count:[0,"itemCount","count"]"#),
            "Aliased input should be array [flags, publicName, declaredName]: {}",
            output
        );
    }

    #[test]
    fn test_inputs_with_transform_format() {
        // Test: Input with transform function -> array format with transform and flags=2
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);
        let transform_fn = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("booleanAttribute"), source_span: None },
            &allocator,
        ));
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("disabled"),
            binding_property_name: Atom::from("disabled"),
            required: false,
            is_signal: false,
            transform_function: Some(transform_fn),
        });

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Expected: {disabled:[2,"disabled","disabled",booleanAttribute]} - array with flags=2 (transform)
        assert!(
            output.contains(r#"disabled:[2,"disabled","disabled",booleanAttribute]"#),
            "Input with transform should be array [flags, publicName, declaredName, transform]: {}",
            output
        );
    }

    #[test]
    fn test_inputs_signal_format() {
        // Test: Signal input -> array format with flags=1
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("border"),
            binding_property_name: Atom::from("border"),
            required: false,
            is_signal: true,
            transform_function: None,
        });

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Expected: {border:[1,"border"]} - array format with flags=1 (signal)
        // For signal inputs with same name, we only need [flags, publicName]
        assert!(
            output.contains(r#"border:[1,"border"]"#),
            "Signal input should be array [flags, publicName]: {}",
            output
        );
        // Should NOT have declared name when same as public name and no transform
        assert!(
            !output.contains(r#"border:[1,"border","border""#),
            "Signal input with same name should not duplicate name: {}",
            output
        );
    }

    #[test]
    fn test_inputs_signal_with_alias_format() {
        // Test: Signal input with alias -> array format with flags=1 and both names
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("borderWidth"),
            binding_property_name: Atom::from("border"),
            required: false,
            is_signal: true,
            transform_function: None,
        });

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Expected: {borderWidth:[1,"border","borderWidth"]} - array with flags=1 and both names
        assert!(
            output.contains(r#"borderWidth:[1,"border","borderWidth"]"#),
            "Signal input with alias should be array [flags, publicName, declaredName]: {}",
            output
        );
    }

    #[test]
    fn test_inputs_signal_with_transform_format() {
        // Test: Signal input with transform -> array format with flags=3 (signal + transform)
        // Note: In practice, signal inputs don't use decorator transforms, but this tests the flag logic
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);
        let transform_fn = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("toNumber"), source_span: None },
            &allocator,
        ));
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("count"),
            binding_property_name: Atom::from("count"),
            required: false,
            is_signal: true,
            transform_function: Some(transform_fn),
        });

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Expected: {count:[3,"count","count",toNumber]} - array with flags=3 (signal + transform)
        assert!(
            output.contains(r#"count:[3,"count","count",toNumber]"#),
            "Signal input with transform should have flags=3: {}",
            output
        );
    }

    #[test]
    fn test_inputs_mixed_types() {
        // Test: Mix of simple, signal, and transform inputs
        let allocator = Allocator::default();
        let mut inputs = Vec::new_in(&allocator);

        // Simple input (flags = 0, uses string format)
        inputs.push(R3InputMetadata::simple(Atom::from("simple")));

        // Signal input (flags = 1)
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("signalInput"),
            binding_property_name: Atom::from("signalInput"),
            required: false,
            is_signal: true,
            transform_function: None,
        });

        // Transform input (flags = 2)
        let transform_fn = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("booleanAttribute"), source_span: None },
            &allocator,
        ));
        inputs.push(R3InputMetadata {
            class_property_name: Atom::from("boolInput"),
            binding_property_name: Atom::from("boolInput"),
            required: false,
            is_signal: false,
            transform_function: Some(transform_fn),
        });

        let result = create_inputs_literal(&allocator, &inputs);
        let emitter = JsEmitter::new();
        let output = result.map(|e| emitter.emit_expression(&e)).unwrap_or_default();

        // Simple input: just string
        assert!(output.contains(r#"simple:"simple""#), "Simple input should be string: {}", output);

        // Signal input: [1, "signalInput"]
        assert!(
            output.contains(r#"signalInput:[1,"signalInput"]"#),
            "Signal input should have flags=1: {}",
            output
        );

        // Transform input: [2, "boolInput", "boolInput", booleanAttribute]
        // Note: The emitter may add newlines in the output for multi-line arrays,
        // so we check for key parts
        assert!(
            output.contains(r#"boolInput:[2,"boolInput","boolInput","#)
                && output.contains("booleanAttribute]"),
            "Transform input should have flags=2: {}",
            output
        );
    }

    #[test]
    fn test_compile_directive_with_features() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("FeatureDirective"), source_span: None },
            &allocator,
        ));

        let metadata = R3DirectiveMetadata {
            name: Atom::from("FeatureDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("[feature]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: true,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: true,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: false,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("features"));
        assert!(output.contains("InheritDefinitionFeature"));
        assert!(output.contains("NgOnChangesFeature"));
        assert!(output.contains("standalone"));
    }

    #[test]
    fn test_compile_directive_with_export_as() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("ExportDirective"), source_span: None },
            &allocator,
        ));

        let mut export_as = Vec::new_in(&allocator);
        export_as.push(Atom::from("myExport"));
        export_as.push(Atom::from("otherExport"));

        let metadata = R3DirectiveMetadata {
            name: Atom::from("ExportDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("[export]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as,
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("exportAs"));
        assert!(output.contains("myExport"));
        assert!(output.contains("otherExport"));
    }

    #[test]
    fn test_compile_directive_with_compound_selector() {
        // Test that compound selectors like "ng-template[body]" are correctly parsed
        // into separate array elements: ["ng-template", "body", ""]
        // This was a bug where the selector was being kept as a single string ["ng-template[body]"]
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("BodyTemplateDirective"), source_span: None },
            &allocator,
        ));

        let metadata = R3DirectiveMetadata {
            name: Atom::from("BodyTemplateDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("ng-template[body]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // Expected: selectors:[[["ng-template","body","",],],]
        // The selector should be parsed into separate array elements, not kept as a single string
        // Note: The emitter may insert newlines, so we check for individual elements
        assert!(
            output.contains(r#""ng-template""#),
            "Expected ng-template element in selector. Got:\n{}",
            output
        );
        assert!(
            output.contains(r#""body""#),
            "Expected body attribute in selector. Got:\n{}",
            output
        );
        // Should NOT contain the selector as a single concatenated string
        assert!(
            !output.contains(r#""ng-template[body]""#),
            "Selector should not be a single concatenated string. Got:\n{}",
            output
        );
    }

    #[test]
    fn test_compile_directive_with_element_and_class_selector() {
        // Test: button.primary should become ["button", 8, "primary"]
        // (8 = CLASS flag)
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("PrimaryButtonDirective"), source_span: None },
            &allocator,
        ));

        let metadata = R3DirectiveMetadata {
            name: Atom::from("PrimaryButtonDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("button.primary")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // Expected: selectors:[[["button",8,"primary",],],]
        // 8 is the CLASS flag
        // Note: The emitter may insert newlines, so we normalize output by removing whitespace
        let normalized = output.replace([' ', '\n', '\t'], "");
        assert!(
            normalized.contains(r#""button",8,"primary""#),
            "Expected selector with CLASS flag (8). Got:\n{}",
            output
        );
    }

    #[test]
    fn test_host_directives_input_output_mappings_use_flat_array() {
        // Issue #67: hostDirectives input/output mappings must be flat arrays
        // ["publicName", "internalName"], NOT objects {publicName: "internalName"}
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("TooltipTrigger"), source_span: None },
            &allocator,
        ));

        let directive_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("BrnTooltipTrigger"), source_span: None },
            &allocator,
        ));

        let mut host_directive_inputs = Vec::new_in(&allocator);
        host_directive_inputs.push((Atom::from("uTooltip"), Atom::from("brnTooltipTrigger")));

        let mut host_directives = Vec::new_in(&allocator);
        host_directives.push(R3HostDirectiveMetadata {
            directive: directive_expr,
            is_forward_reference: false,
            inputs: host_directive_inputs,
            outputs: Vec::new_in(&allocator),
        });

        let metadata = R3DirectiveMetadata {
            name: Atom::from("TooltipTrigger"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("[uTooltip]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives,
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);
        let normalized = output.replace([' ', '\n', '\t'], "");

        // Must contain flat array format: inputs:["brnTooltipTrigger","uTooltip"]
        // (internalName first, then publicName — matching Angular's createHostDirectivesMappingArray)
        assert!(
            normalized.contains(r#"inputs:["brnTooltipTrigger","uTooltip"]"#),
            "Host directive inputs should be flat array [\"internalName\",\"publicName\"]. Got:\n{}",
            output
        );
        // Must NOT contain object format: inputs:{uTooltip:"brnTooltipTrigger"}
        assert!(
            !normalized.contains(r#"inputs:{uTooltip:"brnTooltipTrigger"}"#),
            "Host directive inputs should NOT be object format. Got:\n{}",
            output
        );
    }

    #[test]
    fn test_host_directives_output_mappings_use_flat_array() {
        // Issue #67: output mappings must also be flat arrays
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("MyDirective"), source_span: None },
            &allocator,
        ));

        let directive_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("ClickTracker"), source_span: None },
            &allocator,
        ));

        let mut host_directive_outputs = Vec::new_in(&allocator);
        host_directive_outputs.push((Atom::from("clicked"), Atom::from("trackClick")));

        let mut host_directives = Vec::new_in(&allocator);
        host_directives.push(R3HostDirectiveMetadata {
            directive: directive_expr,
            is_forward_reference: false,
            inputs: Vec::new_in(&allocator),
            outputs: host_directive_outputs,
        });

        let metadata = R3DirectiveMetadata {
            name: Atom::from("MyDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Atom::from("[myDir]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives,
        };

        let result = compile_directive(&allocator, &metadata, 0);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);
        let normalized = output.replace([' ', '\n', '\t'], "");

        // Must contain flat array format: outputs:["trackClick","clicked"]
        // (internalName first, then publicName — matching Angular's createHostDirectivesMappingArray)
        assert!(
            normalized.contains(r#"outputs:["trackClick","clicked"]"#),
            "Host directive outputs should be flat array [\"internalName\",\"publicName\"]. Got:\n{}",
            output
        );
    }
}

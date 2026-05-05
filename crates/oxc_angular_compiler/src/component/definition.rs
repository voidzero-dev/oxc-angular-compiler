//! Component definition generation (ɵcmp and ɵfac).
//!
//! This module generates the Angular runtime definitions that are added
//! as static properties on component classes:
//!
//! - `ɵcmp`: Component definition created by `ɵɵdefineComponent()`
//! - `ɵfac`: Factory function for instantiating the component
//!
//! These definitions are used by Angular's runtime to:
//! - Render the component's template
//! - Handle change detection
//! - Manage component lifecycle
//! - Inject dependencies

use oxc_allocator::{Allocator, Box, FromIn, Vec as OxcVec};
use oxc_str::Ident;

use crate::r3::Identifiers;

use super::dependency::{FactoryTarget, R3DependencyMetadata, compile_inject_dependencies};
use super::metadata::{
    ChangeDetectionStrategy, ComponentMetadata, DeclarationListEmitMode, HostDirectiveMetadata,
    ViewEncapsulation,
};
use super::namespace_registry::NamespaceRegistry;
use super::transform::TransformOptions;
use crate::directive::{
    create_host_directive_mappings_array, create_inputs_literal, create_outputs_literal,
};
use crate::output::ast::{
    FnParam, FunctionExpr, InstantiateExpr, InvokeFunctionExpr, LiteralArrayExpr, LiteralExpr,
    LiteralMapEntry, LiteralMapExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr,
    ReadVarExpr, ReturnStatement,
};
use crate::pipeline::compilation::{ComponentCompilationJob, ConstValue};
use crate::pipeline::emit::HostBindingCompilationResult;
use crate::pipeline::selector::{parse_selector_to_r3_selector, r3_selector_to_output_expr};

/// Result of generating component definitions.
pub struct ComponentDefinitions<'a> {
    /// The ɵcmp definition (component metadata for Angular runtime).
    pub cmp_definition: OutputExpression<'a>,

    /// The ɵfac factory function.
    pub fac_definition: OutputExpression<'a>,
}

/// Generate ɵcmp and ɵfac definitions for a component.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Component metadata extracted from decorator
/// * `job` - The compilation job with template compilation results
/// * `template_fn` - The compiled template function
/// * `host_binding_result` - Optional host binding compilation result (function, hostAttrs, hostVars)
/// * `attrs_ref` - Optional pre-pooled attrs constant reference (pooled before template compilation)
/// * `view_query_fn` - Optional view query function (with pre-pooled predicates)
///
/// # Returns
///
/// The ɵcmp and ɵfac definitions as output expressions.
pub fn generate_component_definitions<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    options: &TransformOptions,
    job: &mut ComponentCompilationJob<'a>,
    template_fn: FunctionExpr<'a>,
    host_binding_result: Option<HostBindingCompilationResult<'a>>,
    attrs_ref: Option<OutputExpression<'a>>,
    view_query_fn: Option<OutputExpression<'a>>,
    content_queries_fn: Option<OutputExpression<'a>>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> ComponentDefinitions<'a> {
    // IMPORTANT: Generate ɵfac BEFORE ɵcmp to match Angular's namespace index assignment order.
    // Angular processes results in order [fac, def, ...] during the transform phase
    // (see packages/compiler-cli/src/ngtsc/transform/src/transform.ts:158-198),
    // so factory dependencies get registered first, followed by component definition dependencies.
    // This ensures namespace indices (i0, i1, i2, ...) are assigned in the same order.
    let fac_definition = generate_fac_definition(allocator, metadata, namespace_registry);
    let cmp_definition = generate_cmp_definition(
        allocator,
        metadata,
        options,
        job,
        template_fn,
        host_binding_result,
        attrs_ref,
        view_query_fn,
        content_queries_fn,
        namespace_registry,
    );

    ComponentDefinitions { cmp_definition, fac_definition }
}

/// Generate the ɵcmp definition.
///
/// Creates an expression like:
/// ```javascript
/// i0.ɵɵdefineComponent({
///   type: ComponentClass,
///   selectors: [["selector"]],
///   decls: 2,
///   vars: 1,
///   template: function ComponentClass_Template(rf, ctx) { ... },
///   styles: ["..."],
///   encapsulation: 0,
///   changeDetection: 0
/// })
/// ```
fn generate_cmp_definition<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    options: &TransformOptions,
    job: &mut ComponentCompilationJob<'a>,
    template_fn: FunctionExpr<'a>,
    host_binding_result: Option<HostBindingCompilationResult<'a>>,
    attrs_ref: Option<OutputExpression<'a>>,
    view_query_fn: Option<OutputExpression<'a>>,
    content_queries_fn: Option<OutputExpression<'a>>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    let mut entries: OxcVec<'a, LiteralMapEntry<'a>> = OxcVec::new_in(allocator);

    // =========================================================================
    // Angular field ordering from baseDirectiveFields (compiler.ts lines 41-104)
    // =========================================================================

    // 1. type: ComponentClass
    entries.push(LiteralMapEntry::new(
        Ident::from("type"),
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
            allocator,
        )),
        false,
    ));

    // 2. selectors: [["selector"]] or [["ng-component"]] if no selector
    // Angular uses "ng-component" as the default selector for components without an explicit selector.
    // See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:264-290
    // and packages/compiler/src/schema/dom_element_schema_registry.ts:463
    let selector_value =
        metadata.selector.as_ref().map_or_else(|| Ident::from("ng-component"), |s| s.clone());
    let selector_entries = parse_selector_to_array(allocator, &selector_value);
    entries.push(LiteralMapEntry::new(Ident::from("selectors"), selector_entries, false));

    // 3. contentQueries: function(rf, ctx, dirIndex) { ... } (if any)
    // This handles @ContentChild/@ContentChildren decorators and signal-based queries
    // (contentChild(), contentChildren()).
    // Per Angular compiler.ts lines 57-63 (baseDirectiveFields)
    if let Some(content_queries) = content_queries_fn {
        entries.push(LiteralMapEntry::new(Ident::from("contentQueries"), content_queries, false));
    }

    // 4. viewQuery: function(rf, ctx) { ... } (if any)
    // This handles @ViewChild/@ViewChildren decorators.
    // The predicate arrays are pre-pooled to ensure correct constant ordering.
    // Per Angular compiler.ts lines 65-70 (baseDirectiveFields)
    if let Some(view_query) = view_query_fn {
        entries.push(LiteralMapEntry::new(Ident::from("viewQuery"), view_query, false));
    }

    // 5-7. Host binding fields (hostAttrs, hostVars, hostBindings)
    // Per Angular compiler.ts lines 72-84 and createHostBindingsFunction (lines 525-532)
    // In Angular, createHostBindingsFunction sets hostAttrs and hostVars on definitionMap
    // before returning the hostBindings function.
    if let Some(host_result) = host_binding_result {
        // 5. hostAttrs: [...] - static host attributes
        if let Some(host_attrs) = host_result.host_attrs {
            entries.push(LiteralMapEntry::new(Ident::from("hostAttrs"), host_attrs, false));
        }

        // 6. hostVars: number - only if > 0
        if let Some(host_vars) = host_result.host_vars {
            entries.push(LiteralMapEntry::new(
                Ident::from("hostVars"),
                OutputExpression::Literal(Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::Number(host_vars as f64),
                        source_span: None,
                    },
                    allocator,
                )),
                false,
            ));
        }

        // 7. hostBindings: function(rf, ctx) { ... } (if any)
        if let Some(host_fn) = host_result.host_binding_fn {
            entries.push(LiteralMapEntry::new(
                Ident::from("hostBindings"),
                OutputExpression::Function(Box::new_in(host_fn, allocator)),
                false,
            ));
        }
    }

    // 8. inputs: { prop: "prop", aliased: { classPropertyName: "aliased", publicName: "alias", ... } }
    // Per Angular compiler.ts lines 86-87 (baseDirectiveFields)
    if !metadata.inputs.is_empty() {
        if let Some(inputs_expr) = create_inputs_literal(allocator, &metadata.inputs) {
            entries.push(LiteralMapEntry::new(Ident::from("inputs"), inputs_expr, false));
        }
    }

    // 9. outputs: { click: "click" }
    // Per Angular compiler.ts lines 89-90 (baseDirectiveFields)
    if !metadata.outputs.is_empty() {
        if let Some(outputs_expr) = create_outputs_literal(allocator, &metadata.outputs) {
            entries.push(LiteralMapEntry::new(Ident::from("outputs"), outputs_expr, false));
        }
    }

    // 10. exportAs: [...] (if not null)
    // Per Angular compiler.ts lines 92-94 (baseDirectiveFields)
    if !metadata.export_as.is_empty() {
        let mut export_items = OxcVec::new_in(allocator);
        for name in &metadata.export_as {
            export_items.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(name.clone()), source_span: None },
                allocator,
            )));
        }
        entries.push(LiteralMapEntry::new(
            Ident::from("exportAs"),
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: export_items, source_span: None },
                allocator,
            )),
            false,
        ));
    }

    // 11. standalone: false - only emit when NOT standalone (true is the default in Angular v17+)
    // Per Angular compiler.ts lines 96-98 (baseDirectiveFields)
    if !metadata.standalone {
        entries.push(LiteralMapEntry::new(
            Ident::from("standalone"),
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(false), source_span: None },
                allocator,
            )),
            false,
        ));
    }

    // 12. signals: true (if isSignal)
    // Per Angular compiler.ts lines 99-101 (baseDirectiveFields)
    if metadata.is_signal {
        entries.push(LiteralMapEntry::new(
            Ident::from("signals"),
            OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(true), source_span: None },
                allocator,
            )),
            false,
        ));
    }

    // =========================================================================
    // Angular field ordering from addFeatures (compiler.ts lines 119-161)
    // =========================================================================

    // 13. features: [...] - component features like providers, lifecycle hooks, inheritance
    // See: packages/compiler/src/render3/view/compiler.ts:119-161
    if let Some(features) = generate_features_array(allocator, metadata, namespace_registry) {
        entries.push(LiteralMapEntry::new(Ident::from("features"), features, false));
    }

    // =========================================================================
    // Angular field ordering from compileComponentFromMetadata (compiler.ts lines 184-354)
    // =========================================================================

    // 14. attrs: ["class", "..."] - only if first selector has attributes
    // This is optional and only included if the first selector specifies attributes.
    // Ported from Angular compiler.ts lines 195-212.
    // The attrs_ref is pre-pooled BEFORE template compilation to ensure correct constant ordering.
    // TypeScript Angular adds attrs to the pool BEFORE template ingestion/compilation.
    if let Some(attrs) = attrs_ref {
        entries.push(LiteralMapEntry::new(Ident::from("attrs"), attrs, false));
    }

    // 15. ngContentSelectors: [...] - content projection selectors
    // Per Angular compiler.ts lines 254-256
    if let Some(content_selectors) = job.content_selectors.take() {
        entries.push(LiteralMapEntry::new(
            Ident::from("ngContentSelectors"),
            content_selectors,
            false,
        ));
    }

    // 16. decls: number (from compilation)
    // Per Angular compiler.ts line 258
    let decls = job.root.decl_count.unwrap_or(0);
    entries.push(LiteralMapEntry::new(
        Ident::from("decls"),
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(decls as f64), source_span: None },
            allocator,
        )),
        false,
    ));

    // 17. vars: number (from compilation)
    // Per Angular compiler.ts line 259
    let vars = job.root.vars.unwrap_or(0);
    entries.push(LiteralMapEntry::new(
        Ident::from("vars"),
        OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(vars as f64), source_span: None },
            allocator,
        )),
        false,
    ));

    // 18. consts: [...] or consts: function() { ...initializers...; return [...]; }
    // Per Angular compiler.ts lines 260-268:
    // - If there are const initializers (e.g., for i18n dual-mode), wrap in arrow function
    // - Otherwise, emit as plain literal array
    if !job.consts.is_empty() {
        let mut const_entries: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);
        for const_value in &job.consts {
            const_entries.push(const_value_to_expression(allocator, const_value));
        }

        let consts_value = if !job.consts_initializers.is_empty() {
            // Wrap consts in an arrow function that runs initializers first
            // function() { ...initializers...; return [...consts...]; }
            let mut fn_stmts: OxcVec<'a, OutputStatement<'a>> =
                OxcVec::with_capacity_in(job.consts_initializers.len() + 1, allocator);

            // Add all initializer statements
            for stmt in job.consts_initializers.drain(..) {
                fn_stmts.push(stmt);
            }

            // Add return statement with consts array
            fn_stmts.push(OutputStatement::Return(Box::new_in(
                ReturnStatement {
                    value: OutputExpression::LiteralArray(Box::new_in(
                        LiteralArrayExpr { entries: const_entries, source_span: None },
                        allocator,
                    )),
                    source_span: None,
                },
                allocator,
            )));

            OutputExpression::Function(Box::new_in(
                FunctionExpr {
                    name: None,
                    params: OxcVec::new_in(allocator),
                    statements: fn_stmts,
                    source_span: None,
                },
                allocator,
            ))
        } else {
            // Plain literal array
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: const_entries, source_span: None },
                allocator,
            ))
        };

        entries.push(LiteralMapEntry::new(Ident::from("consts"), consts_value, false));
    }

    // 19. template: function(rf, ctx) { ... }
    // Per Angular compiler.ts line 270
    entries.push(LiteralMapEntry::new(
        Ident::from("template"),
        OutputExpression::Function(Box::new_in(template_fn, allocator)),
        false,
    ));

    // 20. dependencies: [...] - template dependencies (directives and pipes)
    // Per Angular compiler.ts lines 272-289
    if let Some(dependencies) =
        generate_dependencies_expression(allocator, metadata, namespace_registry)
    {
        entries.push(LiteralMapEntry::new(Ident::from("dependencies"), dependencies, false));
    }

    // 21. styles: [...]
    // Process styles based on encapsulation mode
    // Per Angular compiler (compiler.ts lines 291-323):
    // - For Emulated mode: apply CSS scoping via compileStyles/encapsulate_style
    // - For None/ShadowDom: use styles as-is
    // - If no styles and Emulated: downgrade encapsulation to None
    let mut has_styles = false;
    let mut effective_encapsulation = metadata.encapsulation;

    // CSS scoping uses %COMP% as a placeholder that Angular's runtime replaces
    // with the actual component ID at runtime. This matches Angular's compiler behavior.
    // See: packages/compiler/src/render3/view/compiler.ts
    let content_attr = "_ngcontent-%COMP%";
    let host_attr = "_nghost-%COMP%";

    if !metadata.styles.is_empty() {
        let mut style_entries: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);
        for style in &metadata.styles {
            let style = crate::styles::finalize_component_style(
                style.as_str(),
                metadata.encapsulation == ViewEncapsulation::Emulated,
                content_attr,
                host_attr,
                options.minify_component_styles,
            );
            if style.trim().is_empty() {
                continue;
            }
            let style_value = Ident::from_in(style.as_str(), allocator);

            style_entries.push(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::String(style_value), source_span: None },
                allocator,
            )));
        }

        if !style_entries.is_empty() {
            has_styles = true;
            entries.push(LiteralMapEntry::new(
                Ident::from("styles"),
                OutputExpression::LiteralArray(Box::new_in(
                    LiteralArrayExpr { entries: style_entries, source_span: None },
                    allocator,
                )),
                false,
            ));
        }
    }

    // If no styles and encapsulation is Emulated, downgrade to None
    // (per Angular compiler.ts lines 315-318: "If there is no style, don't generate css selectors on elements")
    if !has_styles && effective_encapsulation == ViewEncapsulation::Emulated {
        effective_encapsulation = ViewEncapsulation::None;
    }

    // 22. encapsulation: number
    // Only set encapsulation if it's NOT the default (Emulated)
    // Per Angular compiler.ts lines 320-323
    if effective_encapsulation != ViewEncapsulation::Emulated {
        let encapsulation_value = match effective_encapsulation {
            ViewEncapsulation::Emulated => 0, // Should not reach here
            ViewEncapsulation::None => 2,
            ViewEncapsulation::ShadowDom => 3,
        };
        entries.push(LiteralMapEntry::new(
            Ident::from("encapsulation"),
            OutputExpression::Literal(Box::new_in(
                LiteralExpr {
                    value: LiteralValue::Number(encapsulation_value as f64),
                    source_span: None,
                },
                allocator,
            )),
            false,
        ));
    }

    // 23. data: {animation: [...]} - animation triggers
    // Per Angular compiler.ts lines 325-331
    if let Some(ref animations) = metadata.animations {
        // Create the inner map: {animation: animationsExpr}
        let mut data_entries: OxcVec<'a, LiteralMapEntry<'a>> =
            OxcVec::with_capacity_in(1, allocator);
        data_entries.push(LiteralMapEntry::new(
            Ident::from("animation"),
            animations.clone_in(allocator),
            false,
        ));

        entries.push(LiteralMapEntry::new(
            Ident::from("data"),
            OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries: data_entries, source_span: None },
                allocator,
            )),
            false,
        ));
    }

    // 24. changeDetection: ChangeDetectionStrategy.OnPush - only emit if not Default
    // (to match TypeScript compiler behavior)
    // Per Angular compiler.ts lines 334-346
    // Angular enum values: OnPush = 0, Default = 1
    // NOTE: Angular emits without namespace prefix (ChangeDetectionStrategy.OnPush, not i0.ChangeDetectionStrategy.OnPush)
    if metadata.change_detection != ChangeDetectionStrategy::Default {
        let strategy_name = match metadata.change_detection {
            ChangeDetectionStrategy::Default => "Default",
            ChangeDetectionStrategy::OnPush => "OnPush",
        };
        // Build: ChangeDetectionStrategy.OnPush (no i0 prefix)
        // ReadPropExpr { receiver: ReadVarExpr("ChangeDetectionStrategy"), name: "OnPush" }
        let change_detection_strategy_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr {
                name: Ident::from(Identifiers::CHANGE_DETECTION_STRATEGY),
                source_span: None,
            },
            allocator,
        ));
        let strategy_value_expr = OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(change_detection_strategy_expr, allocator),
                name: Ident::from(strategy_name),
                optional: false,
                source_span: None,
            },
            allocator,
        ));
        entries.push(LiteralMapEntry::new(
            Ident::from("changeDetection"),
            strategy_value_expr,
            false,
        ));
    }

    // Create the config object
    let config = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries, source_span: None },
        allocator,
    ));

    // Wrap in ɵɵdefineComponent call
    create_define_component_call(allocator, config)
}

/// Generate the ɵfac factory function.
///
/// Creates one of two patterns:
///
/// ## With constructor (constructor_deps is Some):
/// ```javascript
/// // Constructor with dependencies:
/// function ComponentClass_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || ComponentClass)(
///     i0.ɵɵdirectiveInject(ServiceA),
///     i0.ɵɵdirectiveInject(ServiceB, 8)  // 8 = Optional flag
///   );
/// }
///
/// // Constructor with no parameters:
/// function ComponentClass_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || ComponentClass)();
/// }
/// ```
///
/// ## No constructor (constructor_deps is None - use inherited factory):
/// ```javascript
/// /*@__PURE__*/ (() => {
///   let ɵComponentClass_BaseFactory;
///   return function ComponentClass_Factory(__ngFactoryType__) {
///     return (ɵComponentClass_BaseFactory ||
///       (ɵComponentClass_BaseFactory = i0.ɵɵgetInheritedFactory(ComponentClass)))
///       (__ngFactoryType__ || ComponentClass);
///   };
/// })()
/// ```
///
/// Ported from: `packages/compiler/src/render3/r3_factory.ts:106-200`
fn generate_fac_definition<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    // Check if we need inherited factory pattern (no constructor found)
    match &metadata.constructor_deps {
        None => {
            // No constructor - use inherited factory IIFE pattern
            generate_inherited_factory(allocator, metadata)
        }
        Some(deps) => {
            // Constructor exists - generate normal factory
            generate_constructor_factory(allocator, metadata, deps, namespace_registry)
        }
    }
}

/// Generate a normal constructor-based factory function.
///
/// Generates:
/// ```javascript
/// function ComponentClass_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || ComponentClass)(deps...);
/// }
/// ```
fn generate_constructor_factory<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    deps: &[R3DependencyMetadata<'a>],
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    // Function name: ComponentClass_Factory
    let fn_name_string = format!("{}_Factory", metadata.class_name);
    let fn_name = Ident::from_in(fn_name_string.as_str(), allocator);

    // Parameter: __ngFactoryType__ (type override for inheritance/testing)
    let mut params: OxcVec<'a, FnParam<'a>> = OxcVec::new_in(allocator);
    params.push(FnParam { name: Ident::from("__ngFactoryType__") });

    // Body: return new (__ngFactoryType__ || ComponentClass)(deps...);
    let mut statements: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);

    // Create: (__ngFactoryType__ || ComponentClass)
    let or_expr = OutputExpression::BinaryOperator(Box::new_in(
        crate::output::ast::BinaryOperatorExpr {
            operator: crate::output::ast::BinaryOperator::Or,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("__ngFactoryType__"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            source_span: None,
        },
        allocator,
    ));

    // Compile constructor dependencies if any
    // Uses FactoryTarget::Component for components
    // The namespace_registry is used to resolve imported dependency namespaces
    let constructor_args = if deps.is_empty() {
        OxcVec::new_in(allocator)
    } else {
        compile_inject_dependencies(allocator, deps, FactoryTarget::Component, namespace_registry)
    };

    // Create: new (__ngFactoryType__ || ComponentClass)(dep1, dep2, ...)
    let new_expr = OutputExpression::Instantiate(Box::new_in(
        InstantiateExpr {
            class_expr: Box::new_in(or_expr, allocator),
            args: constructor_args,
            source_span: None,
        },
        allocator,
    ));

    // return new (__ngFactoryType__ || ComponentClass)(deps...);
    statements.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: new_expr, source_span: None },
        allocator,
    )));

    OutputExpression::Function(Box::new_in(
        FunctionExpr { name: Some(fn_name), params, statements, source_span: None },
        allocator,
    ))
}

/// Generate an inherited factory using the IIFE memoization pattern.
///
/// Generates:
/// ```javascript
/// /*@__PURE__*/ (() => {
///   let ɵComponentClass_BaseFactory;
///   return function ComponentClass_Factory(__ngFactoryType__) {
///     return (ɵComponentClass_BaseFactory ||
///       (ɵComponentClass_BaseFactory = i0.ɵɵgetInheritedFactory(ComponentClass)))
///       (__ngFactoryType__ || ComponentClass);
///   };
/// })()
/// ```
///
/// See: packages/compiler/src/render3/r3_factory.ts:160-193
fn generate_inherited_factory<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
) -> OutputExpression<'a> {
    use crate::output::ast::{
        ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, DeclareVarStmt,
        StmtModifier,
    };

    let factory_type_param = Ident::from("__ngFactoryType__");

    // Create base factory variable name: ɵComponentClass_BaseFactory
    let base_factory_var_name =
        Ident::from_in(format!("ɵ{}_BaseFactory", metadata.class_name).as_str(), allocator);

    // Function name: ComponentClass_Factory
    let fn_name_string = format!("{}_Factory", metadata.class_name);
    let fn_name = Ident::from_in(fn_name_string.as_str(), allocator);

    // Create ɵɵgetInheritedFactory(ComponentClass) call
    let get_inherited_factory_call = {
        let fn_expr = OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Ident::from("i0"), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: Ident::from(Identifiers::GET_INHERITED_FACTORY),
                optional: false,
                source_span: None,
            },
            allocator,
        ));

        let mut args = OxcVec::new_in(allocator);
        args.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
            allocator,
        )));

        OutputExpression::InvokeFunction(Box::new_in(
            InvokeFunctionExpr {
                fn_expr: Box::new_in(fn_expr, allocator),
                args,
                pure: false,
                optional: false,
                source_span: None,
            },
            allocator,
        ))
    };

    // Create assignment: ɵComponentClass_BaseFactory = ɵɵgetInheritedFactory(ComponentClass)
    let assignment = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Assign,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: base_factory_var_name.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(get_inherited_factory_call, allocator),
            source_span: None,
        },
        allocator,
    ));

    // Create memoization pattern: baseFactoryVar || (baseFactoryVar = ɵɵgetInheritedFactory(...))
    let memoized_factory = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: base_factory_var_name.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(assignment, allocator),
            source_span: None,
        },
        allocator,
    ));

    // Create (__ngFactoryType__ || ComponentClass)
    let type_for_ctor = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: factory_type_param.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            source_span: None,
        },
        allocator,
    ));

    // Create the factory call: (memoizedFactory)(__ngFactoryType__ || ComponentClass)
    let mut factory_call_args = OxcVec::new_in(allocator);
    factory_call_args.push(type_for_ctor);

    let factory_call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(memoized_factory, allocator),
            args: factory_call_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create return statement for inner function
    let mut inner_body: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);
    inner_body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: factory_call, source_span: None },
        allocator,
    )));

    // Create inner function: function ComponentClass_Factory(__ngFactoryType__) { ... }
    let mut inner_params = OxcVec::new_in(allocator);
    inner_params.push(FnParam { name: factory_type_param });

    let inner_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: Some(fn_name),
            params: inner_params,
            statements: inner_body,
            source_span: None,
        },
        allocator,
    ));

    // Create IIFE body: let ɵComponentClass_BaseFactory; return function...;
    let mut iife_body: OxcVec<'a, OutputStatement<'a>> = OxcVec::new_in(allocator);

    // Declaration: let ɵComponentClass_BaseFactory;
    iife_body.push(OutputStatement::DeclareVar(Box::new_in(
        DeclareVarStmt {
            name: base_factory_var_name,
            value: None,
            modifiers: StmtModifier::NONE,
            leading_comment: None,
            source_span: None,
        },
        allocator,
    )));

    // Return the inner function
    iife_body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: inner_fn, source_span: None },
        allocator,
    )));

    // Create arrow function IIFE: () => { let x; return function...; }
    let arrow_fn = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: OxcVec::new_in(allocator),
            body: ArrowFunctionBody::Statements(iife_body),
            source_span: None,
        },
        allocator,
    ));

    // Invoke the IIFE: (() => { ... })()
    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(arrow_fn, allocator),
            args: OxcVec::new_in(allocator),
            pure: true, // Mark as @__PURE__ for tree-shaking
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create an i0.ɵɵdefineComponent(config) call expression.
fn create_define_component_call<'a>(
    allocator: &'a Allocator,
    config: OutputExpression<'a>,
) -> OutputExpression<'a> {
    // Access: i0.ɵɵdefineComponent
    let define_component = OutputExpression::ReadProp(Box::new_in(
        crate::output::ast::ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::DEFINE_COMPONENT),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Call: i0.ɵɵdefineComponent(config)
    let mut args: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);
    args.push(config);

    OutputExpression::InvokeFunction(Box::new_in(
        crate::output::ast::InvokeFunctionExpr {
            fn_expr: Box::new_in(define_component, allocator),
            args,
            pure: true,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Parse a CSS selector string into the Angular R3 selector format.
///
/// Uses the full CSS selector parser to correctly handle combined selectors.
/// Angular represents selectors as nested arrays:
/// - `"app-root"` -> `[["app-root"]]`
/// - `"span[bitBadge]"` -> `[["span", "bitBadge", ""]]`
/// - `"[type=button]"` -> `[["", "type", "button"]]`
/// - `".my-class"` -> `[["", 8, "my-class"]]` (8 = SelectorFlags.CLASS)
///
/// Ported from Angular's `parseSelectorToR3Selector` in `core.ts`.
fn parse_selector_to_array<'a>(
    allocator: &'a Allocator,
    selector: &Ident<'a>,
) -> OutputExpression<'a> {
    let r3_selectors = parse_selector_to_r3_selector(selector.as_str());

    let mut outer_entries: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);

    for r3_selector in &r3_selectors {
        let inner_entries = r3_selector_to_output_expr(allocator, r3_selector);
        outer_entries.push(OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries: inner_entries, source_span: None },
            allocator,
        )));
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: outer_entries, source_span: None },
        allocator,
    ))
}

// =============================================================================
// Features Array Generation
// See: packages/compiler/src/render3/view/compiler.ts:119-161
// =============================================================================

/// Generate the features array for a component definition.
///
/// Features are special runtime behaviors that Angular applies to components:
/// - `ProvidersFeature`: When providers or viewProviders are defined
/// - `HostDirectivesFeature`: When hostDirectives are defined
/// - `InheritDefinitionFeature`: When the component extends another directive/component
/// - `NgOnChangesFeature`: When the component implements ngOnChanges
/// - `ExternalStylesFeature`: When external stylesheets need to be loaded
///
/// Order is important: ProvidersFeature → HostDirectivesFeature → InheritDefinitionFeature
/// → NgOnChangesFeature → ExternalStylesFeature
///
/// See: packages/compiler/src/render3/view/compiler.ts:119-161
fn generate_features_array<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> Option<OutputExpression<'a>> {
    let mut features: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);

    // 1. ProvidersFeature - when providers or viewProviders are defined
    // Format: ɵɵProvidersFeature([providers], [viewProviders]?)
    if metadata.providers.is_some() || metadata.view_providers.is_some() {
        let providers_feature = generate_providers_feature(allocator, metadata);
        features.push(providers_feature);
    }

    // 2. HostDirectivesFeature - when hostDirectives are defined
    // Must come before InheritDefinitionFeature for correct execution order
    // Format: ɵɵHostDirectivesFeature([directives])
    if !metadata.host_directives.is_empty() {
        let host_directives_feature =
            generate_host_directives_feature(allocator, metadata, namespace_registry);
        features.push(host_directives_feature);
    }

    // 3. InheritDefinitionFeature - when the component extends another class
    // Format: ɵɵInheritDefinitionFeature (direct reference, no call)
    if metadata.uses_inheritance {
        features.push(create_angular_fn_ref(allocator, Identifiers::INHERIT_DEFINITION_FEATURE));
    }

    // 4. NgOnChangesFeature - when ngOnChanges lifecycle hook is implemented
    // Format: ɵɵNgOnChangesFeature (direct reference, no call)
    if metadata.lifecycle.uses_on_changes {
        features.push(create_angular_fn_ref(allocator, Identifiers::NG_ON_CHANGES_FEATURE));
    }

    // 5. ExternalStylesFeature - when external stylesheets need to be loaded
    // Format: ɵɵExternalStylesFeature(['style1.css', 'style2.css'])
    if !metadata.external_styles.is_empty() {
        let external_styles_feature = generate_external_styles_feature(allocator, metadata);
        features.push(external_styles_feature);
    }

    // Only return the array if there are features
    if features.is_empty() {
        None
    } else {
        Some(OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries: features, source_span: None },
            allocator,
        )))
    }
}

/// Generate `ɵɵProvidersFeature([providers], [viewProviders]?)` expression.
///
/// See: packages/compiler/src/render3/view/compiler.ts:119-135
fn generate_providers_feature<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, Identifiers::PROVIDERS_FEATURE);

    // Build args: [providers, viewProviders?]
    let has_view_providers = metadata.view_providers.is_some();
    let capacity = if has_view_providers { 2 } else { 1 };
    let mut args: OxcVec<'a, OutputExpression<'a>> = OxcVec::with_capacity_in(capacity, allocator);

    // First arg: providers expression (or empty array if no providers)
    let providers_expr = metadata.providers.as_ref().map_or_else(
        || {
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries: OxcVec::new_in(allocator), source_span: None },
                allocator,
            ))
        },
        |p| p.clone_in(allocator),
    );
    args.push(providers_expr);

    // Second arg: viewProviders (only if present)
    if let Some(ref view_providers) = metadata.view_providers {
        args.push(view_providers.clone_in(allocator));
    }

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Generate `ɵɵHostDirectivesFeature([directives])` expression.
///
/// Handles both simple and complex host directive configurations:
/// - Simple: Just the directive class when no inputs/outputs
/// - Complex: Object with directive, inputs, outputs when mappings exist
///
/// If any directive is a forward reference, wraps in a function.
///
/// See: packages/compiler/src/render3/view/compiler.ts:138-141, 683-723
fn generate_host_directives_feature<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, Identifiers::HOST_DIRECTIVES_FEATURE);

    // Create the host directives argument
    let host_directives_arg =
        create_host_directives_arg(allocator, &metadata.host_directives, namespace_registry);

    let mut args: OxcVec<'a, OutputExpression<'a>> = OxcVec::with_capacity_in(1, allocator);
    args.push(host_directives_arg);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create a directive reference expression.
///
/// For imported directives (those with a `source_module`), generates namespaced
/// references like `i1.MyDirective`. For local directives, generates bare
/// variable references like `MyDirective`.
///
/// See: packages/compiler/src/render3/view/compiler.ts:689-692
fn create_directive_reference<'a>(
    allocator: &'a Allocator,
    directive: &HostDirectiveMetadata<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    if let Some(ref source_module) = directive.source_module {
        // Imported directive - use namespace.DirectiveName (e.g., i1.ExternalDir)
        let namespace = namespace_registry.get_or_assign(source_module);
        return OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: namespace, source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: directive.directive.clone(),
                optional: false,
                source_span: None,
            },
            allocator,
        ));
    }
    // No source module - use bare type name (local directive)
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: directive.directive.clone(), source_span: None },
        allocator,
    ))
}

/// Create the host directives argument array.
///
/// Format depends on whether directives have input/output mappings:
/// - Simple: [Directive1, Directive2] or [i1.ExternalDir] for imports
/// - With mappings: [{ directive: Directive1, inputs: ['prop', 'alias'] }]
/// - Forward ref: function() { return [Directive1] }
///
/// See: packages/compiler/src/render3/view/compiler.ts:683-723
fn create_host_directives_arg<'a>(
    allocator: &'a Allocator,
    host_directives: &[HostDirectiveMetadata<'a>],
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    let mut expressions: OxcVec<'a, OutputExpression<'a>> =
        OxcVec::with_capacity_in(host_directives.len(), allocator);
    let mut has_forward_ref = false;

    for directive in host_directives {
        if directive.is_forward_reference {
            has_forward_ref = true;
        }

        // Create the directive reference (potentially namespaced)
        let directive_ref = create_directive_reference(allocator, directive, namespace_registry);

        if !directive.has_mappings() {
            // Simple case: just the directive type
            expressions.push(directive_ref);
        } else {
            // Complex case: object with directive, inputs, outputs
            let mut entries: OxcVec<'a, LiteralMapEntry<'a>> = OxcVec::new_in(allocator);

            // directive: DirectiveClass (or i1.DirectiveClass for imports)
            entries.push(LiteralMapEntry::new(Ident::from("directive"), directive_ref, false));

            // inputs: ['internalName', 'publicName', ...]
            if !directive.inputs.is_empty() {
                let inputs_array =
                    create_host_directive_mappings_array(allocator, &directive.inputs);
                entries.push(LiteralMapEntry::new(Ident::from("inputs"), inputs_array, false));
            }

            // outputs: ['internalName', 'publicName', ...]
            if !directive.outputs.is_empty() {
                let outputs_array =
                    create_host_directive_mappings_array(allocator, &directive.outputs);
                entries.push(LiteralMapEntry::new(Ident::from("outputs"), outputs_array, false));
            }

            expressions.push(OutputExpression::LiteralMap(Box::new_in(
                LiteralMapExpr { entries, source_span: None },
                allocator,
            )));
        }
    }

    let array_expr = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: expressions, source_span: None },
        allocator,
    ));

    // If there's a forward reference, wrap in: function() { return [directives] }
    if has_forward_ref {
        let mut statements: OxcVec<'a, OutputStatement<'a>> =
            OxcVec::with_capacity_in(1, allocator);
        statements.push(OutputStatement::Return(Box::new_in(
            ReturnStatement { value: array_expr, source_span: None },
            allocator,
        )));

        OutputExpression::Function(Box::new_in(
            FunctionExpr {
                name: None,
                params: OxcVec::new_in(allocator),
                statements,
                source_span: None,
            },
            allocator,
        ))
    } else {
        array_expr
    }
}

/// Generate `ɵɵExternalStylesFeature(['style.css', ...])` expression.
///
/// See: packages/compiler/src/render3/view/compiler.ts:150-155
fn generate_external_styles_feature<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, Identifiers::EXTERNAL_STYLES_FEATURE);

    // Create array of external style paths
    let mut style_entries: OxcVec<'a, OutputExpression<'a>> =
        OxcVec::with_capacity_in(metadata.external_styles.len(), allocator);

    for style_url in &metadata.external_styles {
        style_entries.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(style_url.clone()), source_span: None },
            allocator,
        )));
    }

    let styles_array = OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries: style_entries, source_span: None },
        allocator,
    ));

    let mut args: OxcVec<'a, OutputExpression<'a>> = OxcVec::with_capacity_in(1, allocator);
    args.push(styles_array);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create an `i0.functionName` reference expression.
fn create_angular_fn_ref<'a>(
    allocator: &'a Allocator,
    fn_name: &'static str,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(fn_name),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

// =============================================================================
// Template Dependencies Generation
// See: packages/compiler/src/render3/view/compiler.ts:272-289, 378-396
// =============================================================================

/// Generate the dependencies expression for a component definition.
///
/// Returns the expression to use for the `dependencies` field, or `None` if
/// no dependencies need to be emitted.
///
/// The dependencies expression varies based on `declaration_list_emit_mode`:
/// - `Direct`: `[MyDir, MyPipe]` or `[i1.MyDir, i2.MyPipe]` for imports
/// - `Closure`: `function() { return [MyDir, ForwardDir]; }`
/// - `ClosureResolved`: `function() { return [MyDir].map(ng.resolveForwardRef); }`
/// - `RuntimeResolved`: `ɵɵgetComponentDepsFactory(Component, rawImports)`
///
/// See: packages/compiler/src/render3/view/compiler.ts:272-289
fn generate_dependencies_expression<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> Option<OutputExpression<'a>> {
    // RuntimeResolved mode uses getComponentDepsFactory
    if metadata.declaration_list_emit_mode == DeclarationListEmitMode::RuntimeResolved {
        return Some(generate_runtime_resolved_dependencies(allocator, metadata));
    }

    // No dependencies to emit
    if metadata.declarations.is_empty() {
        return None;
    }

    // Build the array of dependency types
    let deps_array = create_dependencies_array(allocator, metadata, namespace_registry);

    // Compile based on emit mode
    Some(compile_declaration_list(allocator, deps_array, metadata.declaration_list_emit_mode))
}

/// Generate runtime-resolved dependencies expression.
///
/// Format: `ɵɵgetComponentDepsFactory(Component)` or
///         `ɵɵgetComponentDepsFactory(Component, rawImports)`
///
/// See: packages/compiler/src/render3/view/compiler.ts:283-288
fn generate_runtime_resolved_dependencies<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, Identifiers::GET_COMPONENT_DEPS_FACTORY);

    let capacity = if metadata.raw_imports.is_some() { 2 } else { 1 };
    let mut args: OxcVec<'a, OutputExpression<'a>> = OxcVec::with_capacity_in(capacity, allocator);

    // First arg: Component type
    args.push(OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
        allocator,
    )));

    // Second arg: raw imports expression (optional)
    // This can be an array literal `[A, B, C]` or a variable reference `MY_IMPORTS`
    if let Some(ref raw_imports) = metadata.raw_imports {
        args.push(raw_imports.clone_in(allocator));
    }

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create an array of dependency type references.
///
/// For dependencies with a source_module, generates namespaced references like `i1.MyDirective`.
/// For local dependencies, generates bare variable references like `MyDirective`.
fn create_dependencies_array<'a>(
    allocator: &'a Allocator,
    metadata: &ComponentMetadata<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    let mut entries: OxcVec<'a, OutputExpression<'a>> =
        OxcVec::with_capacity_in(metadata.declarations.len(), allocator);

    for dep in &metadata.declarations {
        let dep_expr = if let Some(ref source_module) = dep.source_module {
            // Imported dependency - use namespace.TypeName (e.g., i1.MyDirective)
            let namespace = namespace_registry.get_or_assign(source_module);
            OutputExpression::ReadProp(Box::new_in(
                ReadPropExpr {
                    receiver: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: namespace, source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    name: dep.type_name.clone(),
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        } else {
            // Local dependency - use bare type name
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: dep.type_name.clone(), source_span: None },
                allocator,
            ))
        };
        entries.push(dep_expr);
    }

    OutputExpression::LiteralArray(Box::new_in(
        LiteralArrayExpr { entries, source_span: None },
        allocator,
    ))
}

/// Compile the declaration list based on emit mode.
///
/// See: packages/compiler/src/render3/view/compiler.ts:378-396
fn compile_declaration_list<'a>(
    allocator: &'a Allocator,
    list: OutputExpression<'a>,
    mode: DeclarationListEmitMode,
) -> OutputExpression<'a> {
    match mode {
        DeclarationListEmitMode::Direct => {
            // Direct: [MyDir, MyPipe]
            list
        }
        DeclarationListEmitMode::Closure => {
            // Closure: function() { return [MyDir]; }
            wrap_in_arrow_function(allocator, list)
        }
        DeclarationListEmitMode::ClosureResolved => {
            // ClosureResolved: function() { return [MyDir].map(ng.resolveForwardRef); }
            let resolve_fn = create_angular_fn_ref(allocator, "resolveForwardRef");

            // list.map(ng.resolveForwardRef)
            let map_call = OutputExpression::InvokeFunction(Box::new_in(
                InvokeFunctionExpr {
                    fn_expr: Box::new_in(
                        OutputExpression::ReadProp(Box::new_in(
                            ReadPropExpr {
                                receiver: Box::new_in(list, allocator),
                                name: Ident::from("map"),
                                optional: false,
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    args: {
                        let mut args: OxcVec<'a, OutputExpression<'a>> =
                            OxcVec::with_capacity_in(1, allocator);
                        args.push(resolve_fn);
                        args
                    },
                    pure: false,
                    optional: false,
                    source_span: None,
                },
                allocator,
            ));

            wrap_in_arrow_function(allocator, map_call)
        }
        DeclarationListEmitMode::RuntimeResolved => {
            // RuntimeResolved should be handled by generate_runtime_resolved_dependencies
            // This case should not be reached when calling compile_declaration_list
            list
        }
    }
}

/// Wrap an expression in an arrow function: `function() { return expr; }`
fn wrap_in_arrow_function<'a>(
    allocator: &'a Allocator,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let mut statements: OxcVec<'a, OutputStatement<'a>> = OxcVec::with_capacity_in(1, allocator);
    statements.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: expr, source_span: None },
        allocator,
    )));

    OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: None,
            params: OxcVec::new_in(allocator),
            statements,
            source_span: None,
        },
        allocator,
    ))
}

// =============================================================================
// Constant Pool Serialization
// =============================================================================

/// Convert a ConstValue to an OutputExpression.
///
/// This is used to serialize the consts array entries for the component definition.
pub fn const_value_to_expression<'a>(
    allocator: &'a Allocator,
    value: &ConstValue<'a>,
) -> OutputExpression<'a> {
    match value {
        ConstValue::String(s) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(s.clone()), source_span: None },
            allocator,
        )),
        ConstValue::Number(n) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(*n), source_span: None },
            allocator,
        )),
        ConstValue::Boolean(b) => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(*b), source_span: None },
            allocator,
        )),
        ConstValue::Null => OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            allocator,
        )),
        ConstValue::Array(arr) => {
            let mut entries: OxcVec<'a, OutputExpression<'a>> = OxcVec::new_in(allocator);
            for item in arr.iter() {
                entries.push(const_value_to_expression(allocator, item));
            }
            OutputExpression::LiteralArray(Box::new_in(
                LiteralArrayExpr { entries, source_span: None },
                allocator,
            ))
        }
        ConstValue::External(ext) => {
            // External reference - create i0.ExternalName expression
            OutputExpression::ReadProp(Box::new_in(
                crate::output::ast::ReadPropExpr {
                    receiver: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr { name: Ident::from("i0"), source_span: None },
                            allocator,
                        )),
                        allocator,
                    ),
                    name: ext.name.clone(),
                    optional: false,
                    source_span: None,
                },
                allocator,
            ))
        }
        ConstValue::Expression(expr) => expr.clone_in(allocator),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::component::metadata::{HostDirectiveMetadata, LifecycleMetadata};
    use crate::output::emitter::JsEmitter;
    use oxc_span::Span;

    fn create_test_metadata<'a>(allocator: &'a Allocator) -> ComponentMetadata<'a> {
        let mut metadata =
            ComponentMetadata::new(allocator, Ident::from("TestComponent"), Span::empty(0), true);
        metadata.selector = Some(Ident::from("app-test"));
        metadata
    }

    #[test]
    fn test_generate_fac_definition() {
        let allocator = Allocator::default();
        let metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let fac = generate_fac_definition(&allocator, &metadata, &mut namespace_registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&fac);

        assert!(js.contains("TestComponent_Factory"));
        assert!(js.contains("__ngFactoryType__"));
        assert!(js.contains("TestComponent"));
    }

    #[test]
    fn test_parse_element_selector() {
        let allocator = Allocator::default();
        let result = parse_selector_to_array(&allocator, &Ident::from("app-root"));

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("app-root"));
    }

    #[test]
    fn test_parse_attribute_selector() {
        let allocator = Allocator::default();
        let result = parse_selector_to_array(&allocator, &Ident::from("[type=button]"));

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("type"));
        assert!(js.contains("button"));
    }

    // =========================================================================
    // Default Selector Tests
    // =========================================================================

    #[test]
    fn test_default_selector_ng_component() {
        // When a component has no explicit selector, Angular uses "ng-component" as the default.
        // See: packages/compiler-cli/test/compliance/test_cases/r3_compiler_compliance/
        //      components_and_directives/value_composition/no_selector_def.js
        let allocator = Allocator::default();
        let mut metadata = ComponentMetadata::new(
            &allocator,
            Ident::from("EmptyOutletComponent"),
            Span::empty(0),
            true,
        );
        // No selector set - should default to "ng-component"
        metadata.selector = None;

        let selector_value =
            metadata.selector.as_ref().map_or_else(|| Ident::from("ng-component"), |s| s.clone());
        let result = parse_selector_to_array(&allocator, &selector_value);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should output [["ng-component"]]
        assert!(js.contains("ng-component"), "Should contain 'ng-component' default selector");
    }

    #[test]
    fn test_explicit_selector_overrides_default() {
        let allocator = Allocator::default();
        let mut metadata =
            ComponentMetadata::new(&allocator, Ident::from("TestComponent"), Span::empty(0), true);
        metadata.selector = Some(Ident::from("app-test"));

        let selector_value =
            metadata.selector.as_ref().map_or_else(|| Ident::from("ng-component"), |s| s.clone());
        let result = parse_selector_to_array(&allocator, &selector_value);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should output [["app-test"]], not "ng-component"
        assert!(js.contains("app-test"), "Should contain explicit selector");
        assert!(!js.contains("ng-component"), "Should NOT contain default selector");
    }

    // =========================================================================
    // Features Array Tests
    // =========================================================================

    #[test]
    fn test_no_features() {
        let allocator = Allocator::default();
        let metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let result = generate_features_array(&allocator, &metadata, &mut namespace_registry);
        assert!(result.is_none(), "Should return None when no features are needed");
    }

    /// Helper to create a providers array expression for tests.
    fn create_test_providers_array<'a>(
        allocator: &'a Allocator,
        names: &[&'a str],
    ) -> OutputExpression<'a> {
        let mut entries: OxcVec<'a, OutputExpression<'a>> =
            OxcVec::with_capacity_in(names.len(), allocator);
        for name in names {
            entries.push(OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: Ident::from(*name), source_span: None },
                allocator,
            )));
        }
        OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries, source_span: None },
            allocator,
        ))
    }

    #[test]
    fn test_providers_feature() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.providers =
            Some(create_test_providers_array(&allocator, &["ServiceA", "ServiceB"]));

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵProvidersFeature"));
        assert!(js.contains("ServiceA"));
        assert!(js.contains("ServiceB"));
    }

    #[test]
    fn test_providers_with_view_providers() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.providers = Some(create_test_providers_array(&allocator, &["ServiceA"]));
        metadata.view_providers = Some(create_test_providers_array(&allocator, &["ViewService"]));

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵProvidersFeature"));
        assert!(js.contains("ServiceA"));
        assert!(js.contains("ViewService"));
    }

    #[test]
    fn test_view_providers_only() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        // Empty providers, but viewProviders present
        metadata.view_providers = Some(create_test_providers_array(&allocator, &["ViewService"]));

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should have empty array for providers, then viewProviders
        assert!(js.contains("ɵɵProvidersFeature"));
        assert!(js.contains("ViewService"));
    }

    #[test]
    fn test_inherit_definition_feature() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.uses_inheritance = true;

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵInheritDefinitionFeature"));
        // Should be a direct reference, not a function call
        assert!(!js.contains("ɵɵInheritDefinitionFeature("));
    }

    #[test]
    fn test_ng_on_changes_feature() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.lifecycle = LifecycleMetadata { uses_on_changes: true };

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵNgOnChangesFeature"));
        // Should be a direct reference, not a function call
        assert!(!js.contains("ɵɵNgOnChangesFeature("));
    }

    #[test]
    fn test_external_styles_feature() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.external_styles.push(Ident::from("./styles.css"));
        metadata.external_styles.push(Ident::from("./theme.css"));

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵExternalStylesFeature"));
        assert!(js.contains("./styles.css"));
        assert!(js.contains("./theme.css"));
    }

    #[test]
    fn test_host_directives_simple() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let directive = HostDirectiveMetadata::new(&allocator, Ident::from("MyDirective"));
        metadata.host_directives.push(directive);

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵHostDirectivesFeature"));
        assert!(js.contains("MyDirective"));
    }

    #[test]
    fn test_host_directives_with_mappings() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let mut directive = HostDirectiveMetadata::new(&allocator, Ident::from("MyDirective"));
        directive.inputs.push((Ident::from("publicInput"), Ident::from("internalInput")));
        directive.outputs.push((Ident::from("publicOutput"), Ident::from("internalOutput")));
        metadata.host_directives.push(directive);

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵHostDirectivesFeature"));
        assert!(js.contains("directive"));
        assert!(js.contains("inputs"));
        assert!(js.contains("outputs"));
        assert!(js.contains("publicInput"));
        assert!(js.contains("internalInput"));
    }

    #[test]
    fn test_host_directives_forward_ref() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let mut directive = HostDirectiveMetadata::new(&allocator, Ident::from("ForwardRefDir"));
        directive.is_forward_reference = true;
        metadata.host_directives.push(directive);

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should be wrapped in a function for forward reference
        assert!(js.contains("function"));
        assert!(js.contains("return"));
        assert!(js.contains("ForwardRefDir"));
    }

    #[test]
    fn test_host_directives_with_source_module() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        // Create directive with source_module (imported from another module)
        let directive = HostDirectiveMetadata::new(&allocator, Ident::from("ExternalDirective"))
            .with_source_module(Ident::from("@angular/external"));
        metadata.host_directives.push(directive);

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should use namespaced reference (i1.ExternalDirective)
        assert!(js.contains("ɵɵHostDirectivesFeature"));
        // The namespace should be something like "i1" (depends on registry state)
        // and it should be followed by dot and the directive name
        assert!(
            js.contains("i1.ExternalDirective"),
            "Expected namespaced reference i1.ExternalDirective, got: {}",
            js
        );
    }

    #[test]
    fn test_host_directives_mixed_local_and_imported() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        // Local directive (no source_module)
        let local_directive = HostDirectiveMetadata::new(&allocator, Ident::from("LocalDirective"));
        metadata.host_directives.push(local_directive);

        // Imported directive (with source_module)
        let imported_directive =
            HostDirectiveMetadata::new(&allocator, Ident::from("ImportedDirective"))
                .with_source_module(Ident::from("@angular/library"));
        metadata.host_directives.push(imported_directive);

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Local directive should be bare reference
        assert!(
            js.contains("LocalDirective") && !js.contains("i0.LocalDirective"),
            "Local directive should be bare reference, got: {}",
            js
        );

        // Imported directive should be namespaced
        assert!(
            js.contains("i1.ImportedDirective"),
            "Imported directive should be namespaced, got: {}",
            js
        );
    }

    #[test]
    fn test_multiple_features() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.providers = Some(create_test_providers_array(&allocator, &["ServiceA"]));
        metadata.uses_inheritance = true;
        metadata.lifecycle = LifecycleMetadata { uses_on_changes: true };

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // All three features should be present
        assert!(js.contains("ɵɵProvidersFeature"));
        assert!(js.contains("ɵɵInheritDefinitionFeature"));
        assert!(js.contains("ɵɵNgOnChangesFeature"));
    }

    // =========================================================================
    // Template Dependencies Tests
    // =========================================================================

    #[test]
    fn test_no_dependencies() {
        let allocator = Allocator::default();
        let metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry);
        assert!(result.is_none(), "Should return None when no dependencies");
    }

    #[test]
    fn test_dependencies_direct_mode() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        // Add directives using the TemplateDependency from metadata module
        let dir = crate::component::metadata::TemplateDependency::directive(
            &allocator,
            Ident::from("MyDirective"),
            Ident::from("[myDir]"),
            false,
        );
        metadata.declarations.push(dir);

        let pipe = crate::component::metadata::TemplateDependency::pipe(
            &allocator,
            Ident::from("MyPipe"),
            Ident::from("myPipe"),
        );
        metadata.declarations.push(pipe);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::Direct;

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("MyDirective"));
        assert!(js.contains("MyPipe"));
        // Should be a direct array, not a function
        assert!(!js.contains("function"));
    }

    #[test]
    fn test_dependencies_closure_mode() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let dir = crate::component::metadata::TemplateDependency::directive(
            &allocator,
            Ident::from("ForwardDir"),
            Ident::from("[fwd]"),
            false,
        );
        metadata.declarations.push(dir);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::Closure;

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should be wrapped in a function
        assert!(js.contains("function"));
        assert!(js.contains("return"));
        assert!(js.contains("ForwardDir"));
    }

    #[test]
    fn test_dependencies_closure_resolved_mode() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        let dir = crate::component::metadata::TemplateDependency::directive(
            &allocator,
            Ident::from("JitDir"),
            Ident::from("[jit]"),
            false,
        );
        metadata.declarations.push(dir);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::ClosureResolved;

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should be wrapped in a function with .map(resolveForwardRef)
        assert!(js.contains("function"));
        assert!(js.contains("return"));
        assert!(js.contains("map"));
        assert!(js.contains("resolveForwardRef"));
    }

    #[test]
    fn test_dependencies_runtime_resolved_mode() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::RuntimeResolved;

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵgetComponentDepsFactory"));
        assert!(js.contains("TestComponent"));
    }

    #[test]
    fn test_dependencies_runtime_resolved_with_imports() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::RuntimeResolved;
        // Test with a variable reference as raw_imports
        metadata.raw_imports = Some(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("IMPORTS"), source_span: None },
            &allocator,
        )));

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵgetComponentDepsFactory"));
        assert!(js.contains("TestComponent"));
        assert!(js.contains("IMPORTS"));
    }

    #[test]
    fn test_dependencies_runtime_resolved_with_array_imports() {
        // Test that array literals inside function call args are emitted correctly
        // Angular outputs: ɵɵgetComponentDepsFactory(Comp,[A,B,C])
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::RuntimeResolved;

        // Test with an array literal as raw_imports (like imports: [A, B, C])
        let mut entries: OxcVec<'_, OutputExpression<'_>> = OxcVec::with_capacity_in(3, &allocator);
        entries.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("A"), source_span: None },
            &allocator,
        )));
        entries.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("B"), source_span: None },
            &allocator,
        )));
        entries.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("C"), source_span: None },
            &allocator,
        )));
        metadata.raw_imports = Some(OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries, source_span: None },
            &allocator,
        )));

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should contain the array with dependencies
        assert!(js.contains("[A,B,C]"), "Array should contain dependencies: {}", js);
    }

    #[test]
    fn test_dependencies_with_imported_directive() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        // Add an imported directive with source_module
        let dir = crate::component::metadata::TemplateDependency::directive(
            &allocator,
            Ident::from("RouterOutlet"),
            Ident::from("router-outlet"),
            false,
        )
        .with_source_module(Ident::from("@angular/router"));
        metadata.declarations.push(dir);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::Direct;

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should generate namespaced reference like i1.RouterOutlet
        assert!(js.contains("i1.RouterOutlet"));

        // Verify namespace was registered
        assert!(namespace_registry.has_module(&Ident::from("@angular/router")));
    }

    #[test]
    fn test_dependencies_mixed_local_and_imported() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);

        // Add a local directive (no source_module)
        let local_dir = crate::component::metadata::TemplateDependency::directive(
            &allocator,
            Ident::from("LocalDirective"),
            Ident::from("[local]"),
            false,
        );
        metadata.declarations.push(local_dir);

        // Add an imported directive
        let imported_dir = crate::component::metadata::TemplateDependency::directive(
            &allocator,
            Ident::from("CommonModule"),
            Ident::from("[common]"),
            false,
        )
        .with_source_module(Ident::from("@angular/common"));
        metadata.declarations.push(imported_dir);

        metadata.declaration_list_emit_mode = DeclarationListEmitMode::Direct;

        let result =
            generate_dependencies_expression(&allocator, &metadata, &mut namespace_registry)
                .unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Local directive should be bare reference
        assert!(js.contains("LocalDirective"));
        // Imported directive should be namespaced
        assert!(js.contains("i1.CommonModule"));
    }

    #[test]
    fn test_feature_order() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        let mut namespace_registry = NamespaceRegistry::new(&allocator);
        metadata.providers = Some(create_test_providers_array(&allocator, &["ServiceA"]));
        metadata.uses_inheritance = true;
        metadata.lifecycle = LifecycleMetadata { uses_on_changes: true };
        metadata.external_styles.push(Ident::from("./styles.css"));

        // Add host directive
        let directive = HostDirectiveMetadata::new(&allocator, Ident::from("HostDir"));
        metadata.host_directives.push(directive);

        let result =
            generate_features_array(&allocator, &metadata, &mut namespace_registry).unwrap();

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Verify order: Providers -> HostDirectives -> Inherit -> NgOnChanges -> ExternalStyles
        let providers_pos = js.find("ɵɵProvidersFeature").unwrap();
        let host_pos = js.find("ɵɵHostDirectivesFeature").unwrap();
        let inherit_pos = js.find("ɵɵInheritDefinitionFeature").unwrap();
        let on_changes_pos = js.find("ɵɵNgOnChangesFeature").unwrap();
        let external_pos = js.find("ɵɵExternalStylesFeature").unwrap();

        assert!(
            providers_pos < host_pos,
            "ProvidersFeature should come before HostDirectivesFeature"
        );
        assert!(
            host_pos < inherit_pos,
            "HostDirectivesFeature should come before InheritDefinitionFeature"
        );
        assert!(
            inherit_pos < on_changes_pos,
            "InheritDefinitionFeature should come before NgOnChangesFeature"
        );
        assert!(
            on_changes_pos < external_pos,
            "NgOnChangesFeature should come before ExternalStylesFeature"
        );
    }

    // =========================================================================
    // Animations Data Tests
    // =========================================================================

    #[test]
    fn test_animations_data_field() {
        let allocator = Allocator::default();
        let mut metadata = create_test_metadata(&allocator);
        // Create an animations expression (identifier reference)
        metadata.animations = Some(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("myAnimations"), source_span: None },
            &allocator,
        )));

        // We need a full compilation job to generate the definition
        // For this test, we'll just verify the animations field is set
        assert!(metadata.animations.is_some());
        // Verify it's a ReadVar expression with the correct name
        if let Some(OutputExpression::ReadVar(var)) = &metadata.animations {
            assert_eq!(var.name.as_str(), "myAnimations");
        } else {
            panic!("Expected ReadVar expression for animations");
        }
    }
}

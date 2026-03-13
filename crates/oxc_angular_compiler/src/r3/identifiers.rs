//! Angular runtime identifiers.
//!
//! This module contains all the Angular runtime instruction identifiers
//! that are emitted during template compilation. These correspond to
//! the `Identifiers` class in `@angular/compiler`.
//!
//! All instructions are prefixed with `ɵɵ` (two theta symbols) which marks
//! them as private Angular APIs.

/// The Angular core module name.
pub const CORE: &str = "@angular/core";

/// Angular runtime identifiers used in code generation.
///
/// Each identifier corresponds to a runtime instruction exported from `@angular/core`.
/// The `moduleName` is always `CORE` (`@angular/core`).
pub struct Identifiers;

impl Identifiers {
    // ========================================================================
    // Methods (non-instruction constants)
    // ========================================================================

    /// Factory method name for dependency injection.
    pub const NEW_METHOD: &'static str = "factory";

    /// Transform method name for pipes.
    pub const TRANSFORM_METHOD: &'static str = "transform";

    /// Patched dependencies property name.
    pub const PATCH_DEPS: &'static str = "patchedDeps";

    // ========================================================================
    // Namespace Instructions
    // ========================================================================

    /// Switch to HTML namespace.
    pub const NAMESPACE_HTML: &'static str = "ɵɵnamespaceHTML";

    /// Switch to MathML namespace.
    pub const NAMESPACE_MATH_ML: &'static str = "ɵɵnamespaceMathML";

    /// Switch to SVG namespace.
    pub const NAMESPACE_SVG: &'static str = "ɵɵnamespaceSVG";

    // ========================================================================
    // Element Instructions
    // ========================================================================

    /// Create a self-closing element.
    pub const ELEMENT: &'static str = "ɵɵelement";

    /// Start an element with children.
    pub const ELEMENT_START: &'static str = "ɵɵelementStart";

    /// End an element.
    pub const ELEMENT_END: &'static str = "ɵɵelementEnd";

    /// DOM-only mode: Create a self-closing element.
    pub const DOM_ELEMENT: &'static str = "ɵɵdomElement";

    /// DOM-only mode: Start an element.
    pub const DOM_ELEMENT_START: &'static str = "ɵɵdomElementStart";

    /// DOM-only mode: End an element.
    pub const DOM_ELEMENT_END: &'static str = "ɵɵdomElementEnd";

    /// DOM-only mode: Create an element container.
    pub const DOM_ELEMENT_CONTAINER: &'static str = "ɵɵdomElementContainer";

    /// DOM-only mode: Start an element container.
    pub const DOM_ELEMENT_CONTAINER_START: &'static str = "ɵɵdomElementContainerStart";

    /// DOM-only mode: End an element container.
    pub const DOM_ELEMENT_CONTAINER_END: &'static str = "ɵɵdomElementContainerEnd";

    /// DOM-only mode: Create a template.
    pub const DOM_TEMPLATE: &'static str = "ɵɵdomTemplate";

    /// DOM-only mode: Register an event listener.
    pub const DOM_LISTENER: &'static str = "ɵɵdomListener";

    /// Advance the binding index.
    pub const ADVANCE: &'static str = "ɵɵadvance";

    /// Synthetic host property binding.
    pub const SYNTHETIC_HOST_PROPERTY: &'static str = "ɵɵsyntheticHostProperty";

    /// Synthetic host event listener.
    pub const SYNTHETIC_HOST_LISTENER: &'static str = "ɵɵsyntheticHostListener";

    /// Create an element container (ng-container).
    pub const ELEMENT_CONTAINER: &'static str = "ɵɵelementContainer";

    /// Start an element container.
    pub const ELEMENT_CONTAINER_START: &'static str = "ɵɵelementContainerStart";

    /// End an element container.
    pub const ELEMENT_CONTAINER_END: &'static str = "ɵɵelementContainerEnd";

    // ========================================================================
    // Attribute/Property Binding Instructions
    // ========================================================================

    /// Set an attribute.
    pub const ATTRIBUTE: &'static str = "ɵɵattribute";

    /// Set a class.
    pub const CLASS_PROP: &'static str = "ɵɵclassProp";

    /// Set a style map.
    pub const STYLE_MAP: &'static str = "ɵɵstyleMap";

    /// Set a class map.
    pub const CLASS_MAP: &'static str = "ɵɵclassMap";

    /// Set a style property.
    pub const STYLE_PROP: &'static str = "ɵɵstyleProp";

    /// Set a DOM property.
    pub const DOM_PROPERTY: &'static str = "ɵɵdomProperty";

    /// Set an ARIA property.
    pub const ARIA_PROPERTY: &'static str = "ɵɵariaProperty";

    /// Set a property.
    pub const PROPERTY: &'static str = "ɵɵproperty";

    // ========================================================================
    // Value Interpolation Instructions
    // ========================================================================

    /// Interpolate a value (0 expressions).
    pub const INTERPOLATE: &'static str = "ɵɵinterpolate";

    /// Interpolate a value with 1 expression.
    pub const INTERPOLATE_1: &'static str = "ɵɵinterpolate1";

    /// Interpolate a value with 2 expressions.
    pub const INTERPOLATE_2: &'static str = "ɵɵinterpolate2";

    /// Interpolate a value with 3 expressions.
    pub const INTERPOLATE_3: &'static str = "ɵɵinterpolate3";

    /// Interpolate a value with 4 expressions.
    pub const INTERPOLATE_4: &'static str = "ɵɵinterpolate4";

    /// Interpolate a value with 5 expressions.
    pub const INTERPOLATE_5: &'static str = "ɵɵinterpolate5";

    /// Interpolate a value with 6 expressions.
    pub const INTERPOLATE_6: &'static str = "ɵɵinterpolate6";

    /// Interpolate a value with 7 expressions.
    pub const INTERPOLATE_7: &'static str = "ɵɵinterpolate7";

    /// Interpolate a value with 8 expressions.
    pub const INTERPOLATE_8: &'static str = "ɵɵinterpolate8";

    /// Interpolate a value with 9+ expressions (variadic).
    pub const INTERPOLATE_V: &'static str = "ɵɵinterpolateV";

    // ========================================================================
    // Property Interpolation Instructions (Angular 19 — combined instructions)
    // ========================================================================

    /// Property interpolation (0 expressions, simple stringify).
    pub const PROPERTY_INTERPOLATE: &'static str = "ɵɵpropertyInterpolate";

    /// Property interpolation with 1 expression.
    pub const PROPERTY_INTERPOLATE_1: &'static str = "ɵɵpropertyInterpolate1";

    /// Property interpolation with 2 expressions.
    pub const PROPERTY_INTERPOLATE_2: &'static str = "ɵɵpropertyInterpolate2";

    /// Property interpolation with 3 expressions.
    pub const PROPERTY_INTERPOLATE_3: &'static str = "ɵɵpropertyInterpolate3";

    /// Property interpolation with 4 expressions.
    pub const PROPERTY_INTERPOLATE_4: &'static str = "ɵɵpropertyInterpolate4";

    /// Property interpolation with 5 expressions.
    pub const PROPERTY_INTERPOLATE_5: &'static str = "ɵɵpropertyInterpolate5";

    /// Property interpolation with 6 expressions.
    pub const PROPERTY_INTERPOLATE_6: &'static str = "ɵɵpropertyInterpolate6";

    /// Property interpolation with 7 expressions.
    pub const PROPERTY_INTERPOLATE_7: &'static str = "ɵɵpropertyInterpolate7";

    /// Property interpolation with 8 expressions.
    pub const PROPERTY_INTERPOLATE_8: &'static str = "ɵɵpropertyInterpolate8";

    /// Property interpolation with 9+ expressions (variadic).
    pub const PROPERTY_INTERPOLATE_V: &'static str = "ɵɵpropertyInterpolateV";

    // ========================================================================
    // Attribute Interpolation Instructions (Angular 19 — combined instructions)
    // ========================================================================

    /// Attribute interpolation (0 expressions, simple stringify).
    pub const ATTRIBUTE_INTERPOLATE: &'static str = "ɵɵattributeInterpolate";

    /// Attribute interpolation with 1 expression.
    pub const ATTRIBUTE_INTERPOLATE_1: &'static str = "ɵɵattributeInterpolate1";

    /// Attribute interpolation with 2 expressions.
    pub const ATTRIBUTE_INTERPOLATE_2: &'static str = "ɵɵattributeInterpolate2";

    /// Attribute interpolation with 3 expressions.
    pub const ATTRIBUTE_INTERPOLATE_3: &'static str = "ɵɵattributeInterpolate3";

    /// Attribute interpolation with 4 expressions.
    pub const ATTRIBUTE_INTERPOLATE_4: &'static str = "ɵɵattributeInterpolate4";

    /// Attribute interpolation with 5 expressions.
    pub const ATTRIBUTE_INTERPOLATE_5: &'static str = "ɵɵattributeInterpolate5";

    /// Attribute interpolation with 6 expressions.
    pub const ATTRIBUTE_INTERPOLATE_6: &'static str = "ɵɵattributeInterpolate6";

    /// Attribute interpolation with 7 expressions.
    pub const ATTRIBUTE_INTERPOLATE_7: &'static str = "ɵɵattributeInterpolate7";

    /// Attribute interpolation with 8 expressions.
    pub const ATTRIBUTE_INTERPOLATE_8: &'static str = "ɵɵattributeInterpolate8";

    /// Attribute interpolation with 9+ expressions (variadic).
    pub const ATTRIBUTE_INTERPOLATE_V: &'static str = "ɵɵattributeInterpolateV";

    // ========================================================================
    // Style Prop Interpolation Instructions (Angular 19 — combined instructions)
    // ========================================================================

    /// Style prop interpolation with 1 expression.
    pub const STYLE_PROP_INTERPOLATE_1: &'static str = "ɵɵstylePropInterpolate1";

    /// Style prop interpolation with 2 expressions.
    pub const STYLE_PROP_INTERPOLATE_2: &'static str = "ɵɵstylePropInterpolate2";

    /// Style prop interpolation with 3 expressions.
    pub const STYLE_PROP_INTERPOLATE_3: &'static str = "ɵɵstylePropInterpolate3";

    /// Style prop interpolation with 4 expressions.
    pub const STYLE_PROP_INTERPOLATE_4: &'static str = "ɵɵstylePropInterpolate4";

    /// Style prop interpolation with 5 expressions.
    pub const STYLE_PROP_INTERPOLATE_5: &'static str = "ɵɵstylePropInterpolate5";

    /// Style prop interpolation with 6 expressions.
    pub const STYLE_PROP_INTERPOLATE_6: &'static str = "ɵɵstylePropInterpolate6";

    /// Style prop interpolation with 7 expressions.
    pub const STYLE_PROP_INTERPOLATE_7: &'static str = "ɵɵstylePropInterpolate7";

    /// Style prop interpolation with 8 expressions.
    pub const STYLE_PROP_INTERPOLATE_8: &'static str = "ɵɵstylePropInterpolate8";

    /// Style prop interpolation with 9+ expressions (variadic).
    pub const STYLE_PROP_INTERPOLATE_V: &'static str = "ɵɵstylePropInterpolateV";

    // ========================================================================
    // Style Map Interpolation Instructions (Angular 19 — combined instructions)
    // ========================================================================

    /// Style map interpolation with 1 expression.
    pub const STYLE_MAP_INTERPOLATE_1: &'static str = "ɵɵstyleMapInterpolate1";

    /// Style map interpolation with 2 expressions.
    pub const STYLE_MAP_INTERPOLATE_2: &'static str = "ɵɵstyleMapInterpolate2";

    /// Style map interpolation with 3 expressions.
    pub const STYLE_MAP_INTERPOLATE_3: &'static str = "ɵɵstyleMapInterpolate3";

    /// Style map interpolation with 4 expressions.
    pub const STYLE_MAP_INTERPOLATE_4: &'static str = "ɵɵstyleMapInterpolate4";

    /// Style map interpolation with 5 expressions.
    pub const STYLE_MAP_INTERPOLATE_5: &'static str = "ɵɵstyleMapInterpolate5";

    /// Style map interpolation with 6 expressions.
    pub const STYLE_MAP_INTERPOLATE_6: &'static str = "ɵɵstyleMapInterpolate6";

    /// Style map interpolation with 7 expressions.
    pub const STYLE_MAP_INTERPOLATE_7: &'static str = "ɵɵstyleMapInterpolate7";

    /// Style map interpolation with 8 expressions.
    pub const STYLE_MAP_INTERPOLATE_8: &'static str = "ɵɵstyleMapInterpolate8";

    /// Style map interpolation with 9+ expressions (variadic).
    pub const STYLE_MAP_INTERPOLATE_V: &'static str = "ɵɵstyleMapInterpolateV";

    // ========================================================================
    // Class Map Interpolation Instructions (Angular 19 — combined instructions)
    // ========================================================================

    /// Class map interpolation with 1 expression.
    pub const CLASS_MAP_INTERPOLATE_1: &'static str = "ɵɵclassMapInterpolate1";

    /// Class map interpolation with 2 expressions.
    pub const CLASS_MAP_INTERPOLATE_2: &'static str = "ɵɵclassMapInterpolate2";

    /// Class map interpolation with 3 expressions.
    pub const CLASS_MAP_INTERPOLATE_3: &'static str = "ɵɵclassMapInterpolate3";

    /// Class map interpolation with 4 expressions.
    pub const CLASS_MAP_INTERPOLATE_4: &'static str = "ɵɵclassMapInterpolate4";

    /// Class map interpolation with 5 expressions.
    pub const CLASS_MAP_INTERPOLATE_5: &'static str = "ɵɵclassMapInterpolate5";

    /// Class map interpolation with 6 expressions.
    pub const CLASS_MAP_INTERPOLATE_6: &'static str = "ɵɵclassMapInterpolate6";

    /// Class map interpolation with 7 expressions.
    pub const CLASS_MAP_INTERPOLATE_7: &'static str = "ɵɵclassMapInterpolate7";

    /// Class map interpolation with 8 expressions.
    pub const CLASS_MAP_INTERPOLATE_8: &'static str = "ɵɵclassMapInterpolate8";

    /// Class map interpolation with 9+ expressions (variadic).
    pub const CLASS_MAP_INTERPOLATE_V: &'static str = "ɵɵclassMapInterpolateV";

    // ========================================================================
    // Host Property Instruction (Angular 19 — replaces domProperty)
    // ========================================================================

    /// Host property binding (Angular 19). Angular 20+ uses `ɵɵdomProperty`.
    pub const HOST_PROPERTY: &'static str = "ɵɵhostProperty";

    // ========================================================================
    // Text Instructions
    // ========================================================================

    /// Create a text node.
    pub const TEXT: &'static str = "ɵɵtext";

    /// Interpolate text with 0 expressions.
    pub const TEXT_INTERPOLATE: &'static str = "ɵɵtextInterpolate";

    /// Interpolate text with 1 expression.
    pub const TEXT_INTERPOLATE_1: &'static str = "ɵɵtextInterpolate1";

    /// Interpolate text with 2 expressions.
    pub const TEXT_INTERPOLATE_2: &'static str = "ɵɵtextInterpolate2";

    /// Interpolate text with 3 expressions.
    pub const TEXT_INTERPOLATE_3: &'static str = "ɵɵtextInterpolate3";

    /// Interpolate text with 4 expressions.
    pub const TEXT_INTERPOLATE_4: &'static str = "ɵɵtextInterpolate4";

    /// Interpolate text with 5 expressions.
    pub const TEXT_INTERPOLATE_5: &'static str = "ɵɵtextInterpolate5";

    /// Interpolate text with 6 expressions.
    pub const TEXT_INTERPOLATE_6: &'static str = "ɵɵtextInterpolate6";

    /// Interpolate text with 7 expressions.
    pub const TEXT_INTERPOLATE_7: &'static str = "ɵɵtextInterpolate7";

    /// Interpolate text with 8 expressions.
    pub const TEXT_INTERPOLATE_8: &'static str = "ɵɵtextInterpolate8";

    /// Interpolate text with 9+ expressions (variadic).
    pub const TEXT_INTERPOLATE_V: &'static str = "ɵɵtextInterpolateV";

    // ========================================================================
    // View/Context Instructions
    // ========================================================================

    /// Get the next context (for embedded views).
    pub const NEXT_CONTEXT: &'static str = "ɵɵnextContext";

    /// Reset the view.
    pub const RESET_VIEW: &'static str = "ɵɵresetView";

    /// Restore the view.
    pub const RESTORE_VIEW: &'static str = "ɵɵrestoreView";

    /// Get the current view.
    pub const GET_CURRENT_VIEW: &'static str = "ɵɵgetCurrentView";

    /// Enable bindings.
    pub const ENABLE_BINDINGS: &'static str = "ɵɵenableBindings";

    /// Disable bindings.
    pub const DISABLE_BINDINGS: &'static str = "ɵɵdisableBindings";

    // ========================================================================
    // Template Instructions
    // ========================================================================

    /// Create a template.
    pub const TEMPLATE_CREATE: &'static str = "ɵɵtemplate";

    // ========================================================================
    // Deferred Block Instructions
    // ========================================================================

    /// Create a deferred block.
    pub const DEFER: &'static str = "ɵɵdefer";

    /// Defer when condition is true.
    pub const DEFER_WHEN: &'static str = "ɵɵdeferWhen";

    /// Defer on idle.
    pub const DEFER_ON_IDLE: &'static str = "ɵɵdeferOnIdle";

    /// Defer immediately.
    pub const DEFER_ON_IMMEDIATE: &'static str = "ɵɵdeferOnImmediate";

    /// Defer on timer.
    pub const DEFER_ON_TIMER: &'static str = "ɵɵdeferOnTimer";

    /// Defer on hover.
    pub const DEFER_ON_HOVER: &'static str = "ɵɵdeferOnHover";

    /// Defer on interaction.
    pub const DEFER_ON_INTERACTION: &'static str = "ɵɵdeferOnInteraction";

    /// Defer on viewport.
    pub const DEFER_ON_VIEWPORT: &'static str = "ɵɵdeferOnViewport";

    /// Prefetch when condition is true.
    pub const DEFER_PREFETCH_WHEN: &'static str = "ɵɵdeferPrefetchWhen";

    /// Prefetch on idle.
    pub const DEFER_PREFETCH_ON_IDLE: &'static str = "ɵɵdeferPrefetchOnIdle";

    /// Prefetch immediately.
    pub const DEFER_PREFETCH_ON_IMMEDIATE: &'static str = "ɵɵdeferPrefetchOnImmediate";

    /// Prefetch on timer.
    pub const DEFER_PREFETCH_ON_TIMER: &'static str = "ɵɵdeferPrefetchOnTimer";

    /// Prefetch on hover.
    pub const DEFER_PREFETCH_ON_HOVER: &'static str = "ɵɵdeferPrefetchOnHover";

    /// Prefetch on interaction.
    pub const DEFER_PREFETCH_ON_INTERACTION: &'static str = "ɵɵdeferPrefetchOnInteraction";

    /// Prefetch on viewport.
    pub const DEFER_PREFETCH_ON_VIEWPORT: &'static str = "ɵɵdeferPrefetchOnViewport";

    /// Hydrate when condition is true.
    pub const DEFER_HYDRATE_WHEN: &'static str = "ɵɵdeferHydrateWhen";

    /// Never hydrate.
    pub const DEFER_HYDRATE_NEVER: &'static str = "ɵɵdeferHydrateNever";

    /// Hydrate on idle.
    pub const DEFER_HYDRATE_ON_IDLE: &'static str = "ɵɵdeferHydrateOnIdle";

    /// Hydrate immediately.
    pub const DEFER_HYDRATE_ON_IMMEDIATE: &'static str = "ɵɵdeferHydrateOnImmediate";

    /// Hydrate on timer.
    pub const DEFER_HYDRATE_ON_TIMER: &'static str = "ɵɵdeferHydrateOnTimer";

    /// Hydrate on hover.
    pub const DEFER_HYDRATE_ON_HOVER: &'static str = "ɵɵdeferHydrateOnHover";

    /// Hydrate on interaction.
    pub const DEFER_HYDRATE_ON_INTERACTION: &'static str = "ɵɵdeferHydrateOnInteraction";

    /// Hydrate on viewport.
    pub const DEFER_HYDRATE_ON_VIEWPORT: &'static str = "ɵɵdeferHydrateOnViewport";

    /// Enable timer scheduling for defer.
    pub const DEFER_ENABLE_TIMER_SCHEDULING: &'static str = "ɵɵdeferEnableTimerScheduling";

    // ========================================================================
    // Control Flow Instructions
    // ========================================================================

    /// Create a conditional block.
    pub const CONDITIONAL_CREATE: &'static str = "ɵɵconditionalCreate";

    /// Create a conditional branch.
    pub const CONDITIONAL_BRANCH_CREATE: &'static str = "ɵɵconditionalBranchCreate";

    /// Update a conditional.
    pub const CONDITIONAL: &'static str = "ɵɵconditional";

    /// Update a repeater.
    pub const REPEATER: &'static str = "ɵɵrepeater";

    /// Create a repeater.
    pub const REPEATER_CREATE: &'static str = "ɵɵrepeaterCreate";

    /// Track by index for repeater.
    pub const REPEATER_TRACK_BY_INDEX: &'static str = "ɵɵrepeaterTrackByIndex";

    /// Track by identity for repeater.
    pub const REPEATER_TRACK_BY_IDENTITY: &'static str = "ɵɵrepeaterTrackByIdentity";

    /// Get the component instance.
    pub const COMPONENT_INSTANCE: &'static str = "ɵɵcomponentInstance";

    // ========================================================================
    // Control Instructions
    // ========================================================================

    /// Update a control.
    pub const CONTROL: &'static str = "ɵɵcontrol";

    /// Create a control.
    pub const CONTROL_CREATE: &'static str = "ɵɵcontrolCreate";

    // ========================================================================
    // Animation Instructions
    // ========================================================================

    /// Animation enter listener.
    pub const ANIMATION_ENTER_LISTENER: &'static str = "ɵɵanimateEnterListener";

    /// Animation leave listener.
    pub const ANIMATION_LEAVE_LISTENER: &'static str = "ɵɵanimateLeaveListener";

    /// Animation enter.
    pub const ANIMATION_ENTER: &'static str = "ɵɵanimateEnter";

    /// Animation leave.
    pub const ANIMATION_LEAVE: &'static str = "ɵɵanimateLeave";

    // ========================================================================
    // Pure Function Instructions
    // ========================================================================

    /// Pure function with 0 arguments.
    pub const PURE_FUNCTION_0: &'static str = "ɵɵpureFunction0";

    /// Pure function with 1 argument.
    pub const PURE_FUNCTION_1: &'static str = "ɵɵpureFunction1";

    /// Pure function with 2 arguments.
    pub const PURE_FUNCTION_2: &'static str = "ɵɵpureFunction2";

    /// Pure function with 3 arguments.
    pub const PURE_FUNCTION_3: &'static str = "ɵɵpureFunction3";

    /// Pure function with 4 arguments.
    pub const PURE_FUNCTION_4: &'static str = "ɵɵpureFunction4";

    /// Pure function with 5 arguments.
    pub const PURE_FUNCTION_5: &'static str = "ɵɵpureFunction5";

    /// Pure function with 6 arguments.
    pub const PURE_FUNCTION_6: &'static str = "ɵɵpureFunction6";

    /// Pure function with 7 arguments.
    pub const PURE_FUNCTION_7: &'static str = "ɵɵpureFunction7";

    /// Pure function with 8 arguments.
    pub const PURE_FUNCTION_8: &'static str = "ɵɵpureFunction8";

    /// Pure function with 9+ arguments (variadic).
    pub const PURE_FUNCTION_V: &'static str = "ɵɵpureFunctionV";

    // ========================================================================
    // Pipe Instructions
    // ========================================================================

    /// Create a pipe.
    pub const PIPE: &'static str = "ɵɵpipe";

    /// Pipe bind with 1 argument.
    pub const PIPE_BIND_1: &'static str = "ɵɵpipeBind1";

    /// Pipe bind with 2 arguments.
    pub const PIPE_BIND_2: &'static str = "ɵɵpipeBind2";

    /// Pipe bind with 3 arguments.
    pub const PIPE_BIND_3: &'static str = "ɵɵpipeBind3";

    /// Pipe bind with 4 arguments.
    pub const PIPE_BIND_4: &'static str = "ɵɵpipeBind4";

    /// Pipe bind with 5+ arguments (variadic).
    pub const PIPE_BIND_V: &'static str = "ɵɵpipeBindV";

    // ========================================================================
    // i18n Instructions
    // ========================================================================

    /// Create an i18n block.
    pub const I18N: &'static str = "ɵɵi18n";

    /// i18n attributes.
    pub const I18N_ATTRIBUTES: &'static str = "ɵɵi18nAttributes";

    /// i18n expression.
    pub const I18N_EXP: &'static str = "ɵɵi18nExp";

    /// Start an i18n block.
    pub const I18N_START: &'static str = "ɵɵi18nStart";

    /// End an i18n block.
    pub const I18N_END: &'static str = "ɵɵi18nEnd";

    /// Apply i18n.
    pub const I18N_APPLY: &'static str = "ɵɵi18nApply";

    /// i18n postprocess.
    pub const I18N_POSTPROCESS: &'static str = "ɵɵi18nPostprocess";

    // ========================================================================
    // Projection Instructions
    // ========================================================================

    /// Project content.
    pub const PROJECTION: &'static str = "ɵɵprojection";

    /// Define projection.
    pub const PROJECTION_DEF: &'static str = "ɵɵprojectionDef";

    // ========================================================================
    // Reference Instructions
    // ========================================================================

    /// Get a template reference.
    pub const REFERENCE: &'static str = "ɵɵreference";

    // ========================================================================
    // Injection Instructions
    // ========================================================================

    /// Inject a dependency.
    pub const INJECT: &'static str = "ɵɵinject";

    /// Inject an attribute.
    pub const INJECT_ATTRIBUTE: &'static str = "ɵɵinjectAttribute";

    /// Inject a directive.
    pub const DIRECTIVE_INJECT: &'static str = "ɵɵdirectiveInject";

    /// Invalid factory (for error handling).
    pub const INVALID_FACTORY: &'static str = "ɵɵinvalidFactory";

    /// Invalid factory dependency (for error handling).
    pub const INVALID_FACTORY_DEP: &'static str = "ɵɵinvalidFactoryDep";

    /// Extract a template reference.
    pub const TEMPLATE_REF_EXTRACTOR: &'static str = "ɵɵtemplateRefExtractor";

    /// Forward reference.
    pub const FORWARD_REF: &'static str = "forwardRef";

    /// Resolve forward reference.
    pub const RESOLVE_FORWARD_REF: &'static str = "resolveForwardRef";

    // ========================================================================
    // Replacement/HMR Instructions
    // ========================================================================

    /// Replace metadata.
    pub const REPLACE_METADATA: &'static str = "ɵɵreplaceMetadata";

    /// Get replace metadata URL.
    pub const GET_REPLACE_METADATA_URL: &'static str = "ɵɵgetReplaceMetadataURL";

    // ========================================================================
    // Injectable Definition
    // ========================================================================

    /// Define an injectable.
    pub const DEFINE_INJECTABLE: &'static str = "ɵɵdefineInjectable";

    /// Declare an injectable.
    pub const DECLARE_INJECTABLE: &'static str = "ɵɵngDeclareInjectable";

    /// Injectable declaration type.
    pub const INJECTABLE_DECLARATION: &'static str = "ɵɵInjectableDeclaration";

    // ========================================================================
    // Resolution Instructions
    // ========================================================================

    /// Resolve window.
    pub const RESOLVE_WINDOW: &'static str = "ɵɵresolveWindow";

    /// Resolve document.
    pub const RESOLVE_DOCUMENT: &'static str = "ɵɵresolveDocument";

    /// Resolve body.
    pub const RESOLVE_BODY: &'static str = "ɵɵresolveBody";

    /// Get component deps factory.
    pub const GET_COMPONENT_DEPS_FACTORY: &'static str = "ɵɵgetComponentDepsFactory";

    // ========================================================================
    // Component Definition
    // ========================================================================

    /// Define a component.
    pub const DEFINE_COMPONENT: &'static str = "ɵɵdefineComponent";

    /// Declare a component.
    pub const DECLARE_COMPONENT: &'static str = "ɵɵngDeclareComponent";

    /// Set component scope.
    pub const SET_COMPONENT_SCOPE: &'static str = "ɵɵsetComponentScope";

    /// Change detection strategy enum.
    pub const CHANGE_DETECTION_STRATEGY: &'static str = "ChangeDetectionStrategy";

    /// View encapsulation enum.
    pub const VIEW_ENCAPSULATION: &'static str = "ViewEncapsulation";

    /// Component declaration type.
    pub const COMPONENT_DECLARATION: &'static str = "ɵɵComponentDeclaration";

    // ========================================================================
    // Factory Definition
    // ========================================================================

    /// Factory declaration type.
    pub const FACTORY_DECLARATION: &'static str = "ɵɵFactoryDeclaration";

    /// Declare a factory.
    pub const DECLARE_FACTORY: &'static str = "ɵɵngDeclareFactory";

    /// Factory target enum.
    pub const FACTORY_TARGET: &'static str = "ɵɵFactoryTarget";

    // ========================================================================
    // Directive Definition
    // ========================================================================

    /// Define a directive.
    pub const DEFINE_DIRECTIVE: &'static str = "ɵɵdefineDirective";

    /// Declare a directive.
    pub const DECLARE_DIRECTIVE: &'static str = "ɵɵngDeclareDirective";

    /// Directive declaration type.
    pub const DIRECTIVE_DECLARATION: &'static str = "ɵɵDirectiveDeclaration";

    // ========================================================================
    // Injector Definition
    // ========================================================================

    /// Injector definition type.
    pub const INJECTOR_DEF: &'static str = "ɵɵInjectorDef";

    /// Injector declaration type.
    pub const INJECTOR_DECLARATION: &'static str = "ɵɵInjectorDeclaration";

    /// Define an injector.
    pub const DEFINE_INJECTOR: &'static str = "ɵɵdefineInjector";

    /// Declare an injector.
    pub const DECLARE_INJECTOR: &'static str = "ɵɵngDeclareInjector";

    // ========================================================================
    // NgModule Definition
    // ========================================================================

    /// NgModule declaration type.
    pub const NG_MODULE_DECLARATION: &'static str = "ɵɵNgModuleDeclaration";

    /// Module with providers type.
    pub const MODULE_WITH_PROVIDERS: &'static str = "ModuleWithProviders";

    /// Define an NgModule.
    pub const DEFINE_NG_MODULE: &'static str = "ɵɵdefineNgModule";

    /// Declare an NgModule.
    pub const DECLARE_NG_MODULE: &'static str = "ɵɵngDeclareNgModule";

    /// Set NgModule scope.
    pub const SET_NG_MODULE_SCOPE: &'static str = "ɵɵsetNgModuleScope";

    /// Register NgModule type.
    pub const REGISTER_NG_MODULE_TYPE: &'static str = "ɵɵregisterNgModuleType";

    // ========================================================================
    // Pipe Definition
    // ========================================================================

    /// Pipe declaration type.
    pub const PIPE_DECLARATION: &'static str = "ɵɵPipeDeclaration";

    /// Define a pipe.
    pub const DEFINE_PIPE: &'static str = "ɵɵdefinePipe";

    /// Declare a pipe.
    pub const DECLARE_PIPE: &'static str = "ɵɵngDeclarePipe";

    // ========================================================================
    // Metadata Instructions
    // ========================================================================

    /// Declare class metadata.
    pub const DECLARE_CLASS_METADATA: &'static str = "ɵɵngDeclareClassMetadata";

    /// Declare class metadata async.
    pub const DECLARE_CLASS_METADATA_ASYNC: &'static str = "ɵɵngDeclareClassMetadataAsync";

    /// Set class metadata.
    pub const SET_CLASS_METADATA: &'static str = "ɵsetClassMetadata";

    /// Set class metadata async.
    pub const SET_CLASS_METADATA_ASYNC: &'static str = "ɵsetClassMetadataAsync";

    /// Set class debug info.
    pub const SET_CLASS_DEBUG_INFO: &'static str = "ɵsetClassDebugInfo";

    // ========================================================================
    // Query Instructions
    // ========================================================================

    /// Refresh a query.
    pub const QUERY_REFRESH: &'static str = "ɵɵqueryRefresh";

    /// View query.
    pub const VIEW_QUERY: &'static str = "ɵɵviewQuery";

    /// Load a query.
    pub const LOAD_QUERY: &'static str = "ɵɵloadQuery";

    /// Content query.
    pub const CONTENT_QUERY: &'static str = "ɵɵcontentQuery";

    /// View query signal.
    pub const VIEW_QUERY_SIGNAL: &'static str = "ɵɵviewQuerySignal";

    /// Content query signal.
    pub const CONTENT_QUERY_SIGNAL: &'static str = "ɵɵcontentQuerySignal";

    /// Query advance.
    pub const QUERY_ADVANCE: &'static str = "ɵɵqueryAdvance";

    // ========================================================================
    // Two-Way Binding Instructions
    // ========================================================================

    /// Two-way property binding.
    pub const TWO_WAY_PROPERTY: &'static str = "ɵɵtwoWayProperty";

    /// Two-way binding set.
    pub const TWO_WAY_BINDING_SET: &'static str = "ɵɵtwoWayBindingSet";

    /// Two-way listener.
    pub const TWO_WAY_LISTENER: &'static str = "ɵɵtwoWayListener";

    // ========================================================================
    // Let Declaration Instructions
    // ========================================================================

    /// Declare a let variable.
    pub const DECLARE_LET: &'static str = "ɵɵdeclareLet";

    /// Store a let value.
    pub const STORE_LET: &'static str = "ɵɵstoreLet";

    /// Read a context let.
    pub const READ_CONTEXT_LET: &'static str = "ɵɵreadContextLet";

    // ========================================================================
    // Source Location Instructions
    // ========================================================================

    /// Attach source locations.
    pub const ATTACH_SOURCE_LOCATIONS: &'static str = "ɵɵattachSourceLocations";

    // ========================================================================
    // Feature Instructions
    // ========================================================================

    /// NgOnChanges feature.
    pub const NG_ON_CHANGES_FEATURE: &'static str = "ɵɵNgOnChangesFeature";

    /// Inherit definition feature.
    pub const INHERIT_DEFINITION_FEATURE: &'static str = "ɵɵInheritDefinitionFeature";

    /// Providers feature.
    pub const PROVIDERS_FEATURE: &'static str = "ɵɵProvidersFeature";

    /// Host directives feature.
    pub const HOST_DIRECTIVES_FEATURE: &'static str = "ɵɵHostDirectivesFeature";

    /// External styles feature.
    pub const EXTERNAL_STYLES_FEATURE: &'static str = "ɵɵExternalStylesFeature";

    // ========================================================================
    // Listener Instructions
    // ========================================================================

    /// Register an event listener.
    pub const LISTENER: &'static str = "ɵɵlistener";

    /// Get inherited factory.
    pub const GET_INHERITED_FACTORY: &'static str = "ɵɵgetInheritedFactory";

    // ========================================================================
    // Sanitization Instructions
    // ========================================================================

    /// Sanitize HTML.
    pub const SANITIZE_HTML: &'static str = "ɵɵsanitizeHtml";

    /// Sanitize style.
    pub const SANITIZE_STYLE: &'static str = "ɵɵsanitizeStyle";

    /// Validate attribute.
    pub const VALIDATE_ATTRIBUTE: &'static str = "ɵɵvalidateAttribute";

    /// Sanitize resource URL.
    pub const SANITIZE_RESOURCE_URL: &'static str = "ɵɵsanitizeResourceUrl";

    /// Sanitize script.
    pub const SANITIZE_SCRIPT: &'static str = "ɵɵsanitizeScript";

    /// Sanitize URL.
    pub const SANITIZE_URL: &'static str = "ɵɵsanitizeUrl";

    /// Sanitize URL or resource URL.
    pub const SANITIZE_URL_OR_RESOURCE_URL: &'static str = "ɵɵsanitizeUrlOrResourceUrl";

    /// Trust constant HTML.
    pub const TRUST_CONSTANT_HTML: &'static str = "ɵɵtrustConstantHtml";

    /// Trust constant resource URL.
    pub const TRUST_CONSTANT_RESOURCE_URL: &'static str = "ɵɵtrustConstantResourceUrl";

    // ========================================================================
    // Decorators (non-prefixed, for reflection)
    // ========================================================================

    /// Input decorator.
    pub const INPUT_DECORATOR: &'static str = "Input";

    /// Output decorator.
    pub const OUTPUT_DECORATOR: &'static str = "Output";

    /// ViewChild decorator.
    pub const VIEW_CHILD_DECORATOR: &'static str = "ViewChild";

    /// ViewChildren decorator.
    pub const VIEW_CHILDREN_DECORATOR: &'static str = "ViewChildren";

    /// ContentChild decorator.
    pub const CONTENT_CHILD_DECORATOR: &'static str = "ContentChild";

    /// ContentChildren decorator.
    pub const CONTENT_CHILDREN_DECORATOR: &'static str = "ContentChildren";

    // ========================================================================
    // Type-checking (internal)
    // ========================================================================

    /// Input signal brand write type.
    pub const INPUT_SIGNAL_BRAND_WRITE_TYPE: &'static str = "ɵINPUT_SIGNAL_BRAND_WRITE_TYPE";

    /// Unwrap directive signal inputs.
    pub const UNWRAP_DIRECTIVE_SIGNAL_INPUTS: &'static str = "ɵUnwrapDirectiveSignalInputs";

    /// Unwrap writable signal.
    pub const UNWRAP_WRITABLE_SIGNAL: &'static str = "ɵunwrapWritableSignal";

    /// Assert type.
    pub const ASSERT_TYPE: &'static str = "ɵassertType";
}

/// Returns the interpolation instruction name for the given expression count.
///
/// This is used for value interpolation in property/attribute bindings.
pub fn get_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        0 => Identifiers::INTERPOLATE,
        1 => Identifiers::INTERPOLATE_1,
        2 => Identifiers::INTERPOLATE_2,
        3 => Identifiers::INTERPOLATE_3,
        4 => Identifiers::INTERPOLATE_4,
        5 => Identifiers::INTERPOLATE_5,
        6 => Identifiers::INTERPOLATE_6,
        7 => Identifiers::INTERPOLATE_7,
        8 => Identifiers::INTERPOLATE_8,
        _ => Identifiers::INTERPOLATE_V,
    }
}

/// Returns the text interpolation instruction name for the given expression count.
pub fn get_text_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        0 => Identifiers::TEXT_INTERPOLATE,
        1 => Identifiers::TEXT_INTERPOLATE_1,
        2 => Identifiers::TEXT_INTERPOLATE_2,
        3 => Identifiers::TEXT_INTERPOLATE_3,
        4 => Identifiers::TEXT_INTERPOLATE_4,
        5 => Identifiers::TEXT_INTERPOLATE_5,
        6 => Identifiers::TEXT_INTERPOLATE_6,
        7 => Identifiers::TEXT_INTERPOLATE_7,
        8 => Identifiers::TEXT_INTERPOLATE_8,
        _ => Identifiers::TEXT_INTERPOLATE_V,
    }
}

/// Returns the pure function instruction name for the given argument count.
pub fn get_pure_function_instruction(arg_count: usize) -> &'static str {
    match arg_count {
        0 => Identifiers::PURE_FUNCTION_0,
        1 => Identifiers::PURE_FUNCTION_1,
        2 => Identifiers::PURE_FUNCTION_2,
        3 => Identifiers::PURE_FUNCTION_3,
        4 => Identifiers::PURE_FUNCTION_4,
        5 => Identifiers::PURE_FUNCTION_5,
        6 => Identifiers::PURE_FUNCTION_6,
        7 => Identifiers::PURE_FUNCTION_7,
        8 => Identifiers::PURE_FUNCTION_8,
        _ => Identifiers::PURE_FUNCTION_V,
    }
}

/// Returns the pipe bind instruction name for the given argument count.
pub fn get_pipe_bind_instruction(arg_count: usize) -> &'static str {
    match arg_count {
        1 => Identifiers::PIPE_BIND_1,
        2 => Identifiers::PIPE_BIND_2,
        3 => Identifiers::PIPE_BIND_3,
        4 => Identifiers::PIPE_BIND_4,
        _ => Identifiers::PIPE_BIND_V,
    }
}

/// Returns the property interpolation instruction name for the given expression count (Angular 19).
pub fn get_property_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        0 => Identifiers::PROPERTY_INTERPOLATE,
        1 => Identifiers::PROPERTY_INTERPOLATE_1,
        2 => Identifiers::PROPERTY_INTERPOLATE_2,
        3 => Identifiers::PROPERTY_INTERPOLATE_3,
        4 => Identifiers::PROPERTY_INTERPOLATE_4,
        5 => Identifiers::PROPERTY_INTERPOLATE_5,
        6 => Identifiers::PROPERTY_INTERPOLATE_6,
        7 => Identifiers::PROPERTY_INTERPOLATE_7,
        8 => Identifiers::PROPERTY_INTERPOLATE_8,
        _ => Identifiers::PROPERTY_INTERPOLATE_V,
    }
}

/// Returns the attribute interpolation instruction name for the given expression count (Angular 19).
pub fn get_attribute_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        0 => Identifiers::ATTRIBUTE_INTERPOLATE,
        1 => Identifiers::ATTRIBUTE_INTERPOLATE_1,
        2 => Identifiers::ATTRIBUTE_INTERPOLATE_2,
        3 => Identifiers::ATTRIBUTE_INTERPOLATE_3,
        4 => Identifiers::ATTRIBUTE_INTERPOLATE_4,
        5 => Identifiers::ATTRIBUTE_INTERPOLATE_5,
        6 => Identifiers::ATTRIBUTE_INTERPOLATE_6,
        7 => Identifiers::ATTRIBUTE_INTERPOLATE_7,
        8 => Identifiers::ATTRIBUTE_INTERPOLATE_8,
        _ => Identifiers::ATTRIBUTE_INTERPOLATE_V,
    }
}

/// Returns the style prop interpolation instruction name for the given expression count (Angular 19).
pub fn get_style_prop_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        1 => Identifiers::STYLE_PROP_INTERPOLATE_1,
        2 => Identifiers::STYLE_PROP_INTERPOLATE_2,
        3 => Identifiers::STYLE_PROP_INTERPOLATE_3,
        4 => Identifiers::STYLE_PROP_INTERPOLATE_4,
        5 => Identifiers::STYLE_PROP_INTERPOLATE_5,
        6 => Identifiers::STYLE_PROP_INTERPOLATE_6,
        7 => Identifiers::STYLE_PROP_INTERPOLATE_7,
        8 => Identifiers::STYLE_PROP_INTERPOLATE_8,
        _ => Identifiers::STYLE_PROP_INTERPOLATE_V,
    }
}

/// Returns the style map interpolation instruction name for the given expression count (Angular 19).
pub fn get_style_map_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        1 => Identifiers::STYLE_MAP_INTERPOLATE_1,
        2 => Identifiers::STYLE_MAP_INTERPOLATE_2,
        3 => Identifiers::STYLE_MAP_INTERPOLATE_3,
        4 => Identifiers::STYLE_MAP_INTERPOLATE_4,
        5 => Identifiers::STYLE_MAP_INTERPOLATE_5,
        6 => Identifiers::STYLE_MAP_INTERPOLATE_6,
        7 => Identifiers::STYLE_MAP_INTERPOLATE_7,
        8 => Identifiers::STYLE_MAP_INTERPOLATE_8,
        _ => Identifiers::STYLE_MAP_INTERPOLATE_V,
    }
}

/// Returns the class map interpolation instruction name for the given expression count (Angular 19).
pub fn get_class_map_interpolate_instruction(expr_count: usize) -> &'static str {
    match expr_count {
        1 => Identifiers::CLASS_MAP_INTERPOLATE_1,
        2 => Identifiers::CLASS_MAP_INTERPOLATE_2,
        3 => Identifiers::CLASS_MAP_INTERPOLATE_3,
        4 => Identifiers::CLASS_MAP_INTERPOLATE_4,
        5 => Identifiers::CLASS_MAP_INTERPOLATE_5,
        6 => Identifiers::CLASS_MAP_INTERPOLATE_6,
        7 => Identifiers::CLASS_MAP_INTERPOLATE_7,
        8 => Identifiers::CLASS_MAP_INTERPOLATE_8,
        _ => Identifiers::CLASS_MAP_INTERPOLATE_V,
    }
}

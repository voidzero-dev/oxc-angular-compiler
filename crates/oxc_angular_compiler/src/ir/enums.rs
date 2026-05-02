//! IR operation and expression kind enumerations.
//!
//! Ported from Angular's `template/pipeline/ir/src/enums.ts`.

/// Distinguishes different kinds of IR operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum OpKind {
    /// A special operation type for list endpoints.
    ListEnd = 0,
    /// An operation wrapping an output AST statement.
    Statement,
    /// An operation declaring a semantic variable.
    Variable,

    // Element operations
    /// Begin rendering an element.
    ElementStart,
    /// Render an element with no children.
    Element,
    /// End rendering an element.
    ElementEnd,
    /// An embedded view declaration.
    Template,

    // Container operations
    /// Begin an ng-container.
    ContainerStart,
    /// An ng-container with no children.
    Container,
    /// End an ng-container.
    ContainerEnd,

    // Binding control
    /// Disable binding for descendants of non-bindable nodes.
    DisableBindings,
    /// Re-enable binding.
    EnableBindings,

    // Conditional operations
    /// Create a conditional instruction.
    ConditionalCreate,
    /// Create a conditional branch instruction (for branches after the first in @if/@switch).
    ConditionalBranchCreate,
    /// Conditionally render a template.
    Conditional,

    // Text operations
    /// Render a text node.
    Text,
    /// Interpolate text into a text node.
    InterpolateText,

    // Event operations
    /// Declare an event listener.
    Listener,

    // Binding operations
    /// An intermediate binding (not yet processed).
    Binding,
    /// Bind to an element property.
    Property,
    /// Bind to a style property.
    StyleProp,
    /// Bind to a class property.
    ClassProp,
    /// Bind to styles.
    StyleMap,
    /// Bind to classes.
    ClassMap,
    /// Advance the runtime's implicit slot context.
    Advance,

    // Pipe operations
    /// Instantiate a pipe.
    Pipe,

    // Attribute operations
    /// Associate an attribute with an element.
    Attribute,
    /// An extracted attribute for consts array.
    ExtractedAttribute,

    // Defer operations
    /// Configure a @defer block.
    Defer,
    /// Control when a @defer loads.
    DeferOn,
    /// Control @defer with a custom condition.
    DeferWhen,

    // I18n operations
    /// An i18n message for consts array.
    I18nMessage,
    /// Native DOM property binding.
    DomProperty,

    // Namespace operations
    /// Change namespace (HTML, SVG, Math).
    Namespace,

    // Projection operations
    /// Configure content projection.
    ProjectionDef,
    /// Create a content projection slot.
    Projection,

    // Repeater operations
    /// Create a repeater instruction.
    RepeaterCreate,
    /// Update a repeater.
    Repeater,

    // Two-way binding operations
    /// Two-way property binding.
    TwoWayProperty,
    /// Two-way listener.
    TwoWayListener,

    // Let declaration operations
    /// Initialize a @let slot.
    DeclareLet,
    /// Store current @let value.
    StoreLet,

    // I18n block operations
    /// Start an i18n block.
    I18nStart,
    /// Self-closing i18n on a single element.
    I18n,
    /// End an i18n block.
    I18nEnd,
    /// An expression in an i18n message.
    I18nExpression,
    /// Apply i18n expressions.
    I18nApply,

    // ICU operations
    /// Create an ICU expression.
    IcuStart,
    /// Update an ICU expression.
    IcuEnd,
    /// A placeholder in an ICU expression.
    IcuPlaceholder,
    /// An i18n context for message generation.
    I18nContext,
    /// I18n attributes on an element.
    I18nAttributes,

    // Source location
    /// Attach source location to an element.
    SourceLocation,

    // Animation operations
    /// Bind animation CSS classes.
    Animation,
    /// Animation string binding.
    AnimationString,
    /// Animation binding.
    AnimationBinding,
    /// Animation listener.
    AnimationListener,

    // Control operations
    /// Bind to a field property.
    Control,
    /// Create a control binding instruction (for specialized control directives on form controls).
    ControlCreate,
}

/// Distinguishes different kinds of IR expressions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ExpressionKind {
    /// Read a variable in lexical scope.
    LexicalRead,
    /// Reference to current view context.
    Context,
    /// Reference to view context for track functions.
    TrackContext,
    /// Read a variable declared in VariableOp.
    ReadVariable,
    /// Navigate to next view context.
    NextContext,
    /// Retrieve a local reference value.
    Reference,
    /// Store a @let declaration value.
    StoreLet,
    /// Read a @let declaration from context.
    ContextLetReference,
    /// Snapshot current view context.
    GetCurrentView,
    /// Restore a snapshotted view.
    RestoreView,
    /// Reset view context after RestoreView.
    ResetView,
    /// Pure function with change-detected args.
    PureFunctionExpr,
    /// Parameter to pure function.
    PureFunctionParameterExpr,
    /// Pipe transformation binding.
    PipeBinding,
    /// Pipe with variable arguments.
    PipeBindingVariadic,
    /// Safe property read needing expansion.
    SafePropertyRead,
    /// Safe keyed read needing expansion.
    SafeKeyedRead,
    /// Safe function call needing expansion.
    SafeInvokeFunction,
    /// Intermediate ternary from safe read.
    SafeTernaryExpr,
    /// Empty expression to be stripped.
    EmptyExpr,
    /// Assignment to temporary variable.
    AssignTemporaryExpr,
    /// Reference to temporary variable.
    ReadTemporaryExpr,
    /// Emit a literal slot index.
    SlotLiteralExpr,
    /// Test for conditional op.
    ConditionalCase,
    /// Auto-extract to component const array.
    ConstCollected,
    /// Reference to const array index.
    ConstReference,
    /// Set value of two-way binding.
    TwoWayBindingSet,
    /// Interpolation expression.
    Interpolation,
    /// Binary operator expression.
    Binary,
    /// Ternary conditional expression.
    Ternary,
    /// Property read with resolved receiver (used after name resolution).
    ResolvedPropertyRead,
    /// Binary expression with resolved sub-expressions (used after name resolution).
    ResolvedBinary,
    /// Function call with resolved receiver and/or arguments (used after name resolution).
    ResolvedCall,
    /// Keyed read with resolved receiver (used after name resolution).
    ResolvedKeyedRead,
    /// Safe property read with resolved receiver (used after name resolution).
    ResolvedSafePropertyRead,
    /// Derived literal array for pure function bodies.
    DerivedLiteralArray,
    /// Derived literal map for pure function bodies.
    DerivedLiteralMap,
    /// Literal array with IR expression elements.
    LiteralArray,
    /// Literal map (object) with IR expression values.
    LiteralMap,
    /// Logical NOT expression (!expr).
    Not,
    /// Unary operator expression (+expr or -expr).
    Unary,
    /// Typeof expression (typeof expr).
    Typeof,
    /// Void expression (void expr).
    Void,
    /// Template literal with resolved expressions (used after name resolution).
    ResolvedTemplateLiteral,
    /// Arrow function expression.
    ArrowFunction,
    /// Parenthesized expression.
    Parenthesized,
}

/// Flags for semantic variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct VariableFlags(u8);

impl VariableFlags {
    /// No flags.
    pub const NONE: Self = Self(0b0000);
    /// Always inline this variable.
    pub const ALWAYS_INLINE: Self = Self(0b0001);

    /// Check if a flag is set.
    pub fn contains(self, other: Self) -> bool {
        (self.0 & other.0) == other.0
    }
}

/// Kinds of semantic variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SemanticVariableKind {
    /// Context of a particular view.
    Context,
    /// Identifier in lexical scope.
    Identifier,
    /// Saved state for listener handlers.
    SavedView,
    /// Alias from embedded view (e.g., @for).
    Alias,
}

/// Compatibility mode for template compilation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompatibilityMode {
    /// Normal compilation mode.
    #[default]
    Normal,
    /// Match TemplateDefinitionBuilder output.
    TemplateDefinitionBuilder,
}

/// Types of bindings applied to elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BindingKind {
    /// Static attributes.
    Attribute,
    /// Class bindings.
    ClassName,
    /// Style bindings.
    StyleProperty,
    /// Dynamic property bindings.
    Property,
    /// Template property/attribute bindings.
    Template,
    /// Internationalized attributes.
    I18n,
    /// Legacy animation bindings.
    LegacyAnimation,
    /// Two-way property binding.
    TwoWayProperty,
    /// Animation binding.
    Animation,
}

/// Resolution time for i18n params.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I18nParamResolutionTime {
    /// Resolve at message creation.
    Creation,
    /// Resolve during post-processing (ICU).
    Postprocessing,
}

/// Contexts for i18n expression usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I18nExpressionFor {
    /// Used as value in i18n block.
    I18nText,
    /// Used in a binding.
    I18nAttribute,
}

/// Flags for i18n param values.
///
/// These flags determine how an i18n param value is serialized into the final map.
/// Multiple flags can be combined using the `with` method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct I18nParamValueFlags(u8);

impl I18nParamValueFlags {
    /// No flags.
    pub const NONE: Self = Self(0b0000);
    /// Element tag.
    pub const ELEMENT_TAG: Self = Self(0b0001);
    /// Template tag.
    pub const TEMPLATE_TAG: Self = Self(0b0010);
    /// Opening tag.
    pub const OPEN_TAG: Self = Self(0b0100);
    /// Closing tag.
    pub const CLOSE_TAG: Self = Self(0b1000);
    /// Expression index.
    pub const EXPRESSION_INDEX: Self = Self(0b10000);

    /// Combine this flag with another flag.
    #[must_use]
    pub const fn with(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Check if this flag set contains a specific flag.
    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) == flag.0
    }

    /// Get the raw value of the flags.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// Create flags from raw bits.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    /// Remove a flag from this flag set.
    #[must_use]
    pub const fn without(self, flag: Self) -> Self {
        Self(self.0 & !flag.0)
    }
}

/// Active namespace (HTML, SVG, Math).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Namespace {
    /// HTML namespace.
    #[default]
    Html,
    /// SVG namespace.
    Svg,
    /// MathML namespace.
    Math,
}

/// Types of @defer triggers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeferTriggerKind {
    /// Idle trigger.
    Idle,
    /// Immediate trigger.
    Immediate,
    /// Timer trigger.
    Timer,
    /// Hover trigger.
    Hover,
    /// Interaction trigger.
    Interaction,
    /// Viewport trigger.
    Viewport,
    /// Never trigger.
    Never,
}

/// Kinds of i18n contexts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I18nContextKind {
    /// Root i18n block.
    RootI18n,
    /// ICU expression.
    Icu,
    /// Attribute.
    Attr,
}

/// Kinds of templates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateKind {
    /// ng-template.
    NgTemplate,
    /// Structural directive.
    Structural,
    /// Block (control flow).
    Block,
}

/// Kinds of animations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationKind {
    /// Enter animation.
    Enter,
    /// Leave animation.
    Leave,
}

impl AnimationKind {
    /// Returns the phase string used in legacy animation event names ("start" or "done").
    pub fn legacy_phase_str(self) -> &'static str {
        match self {
            AnimationKind::Enter => "start",
            AnimationKind::Leave => "done",
        }
    }
}

/// Kinds of animation bindings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationBindingKind {
    /// String animation.
    String,
    /// Value animation.
    Value,
}

/// Modifier kinds for defer blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeferOpModifierKind {
    /// No modifier.
    #[default]
    None,
    /// Prefetch modifier.
    Prefetch,
    /// Hydrate modifier.
    Hydrate,
}

/// Flags for TDeferDetails at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TDeferDetailsFlags(u8);

impl TDeferDetailsFlags {
    /// Default flags.
    pub const DEFAULT: Self = Self(0);
    /// Has hydrate triggers.
    pub const HAS_HYDRATE_TRIGGERS: Self = Self(1 << 0);
}

#[cfg(test)]
mod tests {
    use super::AnimationKind;

    #[test]
    fn animation_kind_enter_maps_to_start() {
        assert_eq!(AnimationKind::Enter.legacy_phase_str(), "start");
    }

    #[test]
    fn animation_kind_leave_maps_to_done() {
        assert_eq!(AnimationKind::Leave.legacy_phase_str(), "done");
    }
}

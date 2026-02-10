//! IR operation definitions.
//!
//! Operations are the fundamental units of the Angular template IR.
//! They are divided into two categories:
//! - Create operations: Executed once during component initialization
//! - Update operations: Executed on every change detection cycle
//!
//! Ported from Angular's `template/pipeline/ir/src/ops/*.ts`.

use std::ptr::NonNull;

use oxc_allocator::{Box, Vec};
use oxc_span::{Atom, Span};

use super::enums::{
    AnimationBindingKind, AnimationKind, BindingKind, DeferOpModifierKind, DeferTriggerKind,
    I18nContextKind, I18nExpressionFor, I18nParamResolutionTime, Namespace, OpKind,
    SemanticVariableKind, TemplateKind, VariableFlags,
};
use super::expression::{ConditionalCaseExpr, IrExpression};
use crate::ast::r3::SecurityContext;
use crate::output::ast::OutputExpression;

/// Cross-reference ID for linking operations across views.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct XrefId(pub u32);

impl XrefId {
    /// Creates a new XrefId.
    pub fn new(id: u32) -> Self {
        Self(id)
    }
}

/// Slot ID for runtime slot allocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SlotId(pub u32);

/// Consume ID for tracking value consumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ConsumeId(pub u32);

/// I18n slot allocation result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I18nSlotHandle {
    /// Single slot.
    Single(SlotId),
    /// Range of slots (start, end).
    Range(SlotId, SlotId),
}

/// I18n placeholder data for elements, templates, and blocks.
///
/// This corresponds to Angular's `i18n.TagPlaceholder` and `i18n.BlockPlaceholder`
/// which both have `startName` and `closeName` properties for paired tags.
///
/// For self-closing/void elements, `close_name` will be `None`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct I18nPlaceholder<'a> {
    /// The placeholder name for the opening tag (e.g., "START_TAG_DIV").
    pub start_name: Atom<'a>,
    /// The placeholder name for the closing tag (e.g., "CLOSE_TAG_DIV").
    /// `None` for self-closing/void elements.
    pub close_name: Option<Atom<'a>>,
}

impl<'a> I18nPlaceholder<'a> {
    /// Creates a new I18nPlaceholder.
    pub fn new(start_name: Atom<'a>, close_name: Option<Atom<'a>>) -> Self {
        Self { start_name, close_name }
    }

    /// Creates a placeholder for a self-closing/void element.
    pub fn self_closing(start_name: Atom<'a>) -> Self {
        Self { start_name, close_name: None }
    }
}

/// Trait for all IR operations.
pub trait Op: Sized {
    /// Returns the kind of this operation.
    fn kind(&self) -> OpKind;

    /// Returns the previous operation in the list.
    fn prev(&self) -> Option<NonNull<Self>>;

    /// Returns the next operation in the list.
    fn next(&self) -> Option<NonNull<Self>>;

    /// Sets the previous operation in the list.
    fn set_prev(&mut self, prev: Option<NonNull<Self>>);

    /// Sets the next operation in the list.
    fn set_next(&mut self, next: Option<NonNull<Self>>);

    /// Returns the debug info for this operation.
    fn debug_string(&self) -> &str {
        ""
    }
}

// ============================================================================
// Create Operations
// ============================================================================

/// Base fields common to all create operations.
#[derive(Debug)]
pub struct CreateOpBase<'a> {
    /// Link to previous operation.
    pub prev: Option<NonNull<CreateOp<'a>>>,
    /// Link to next operation.
    pub next: Option<NonNull<CreateOp<'a>>>,
    /// Source span for debugging.
    pub source_span: Option<Span>,
}

impl<'a> Default for CreateOpBase<'a> {
    fn default() -> Self {
        Self { prev: None, next: None, source_span: None }
    }
}

/// Create-time operation.
#[derive(Debug)]
pub enum CreateOp<'a> {
    /// List end marker.
    ListEnd(ListEndOp<'a>),
    /// Start rendering an element.
    ElementStart(ElementStartOp<'a>),
    /// Render an element with no children.
    Element(ElementOp<'a>),
    /// End rendering an element.
    ElementEnd(ElementEndOp<'a>),
    /// Create a template.
    Template(TemplateOp<'a>),
    /// Start an ng-container.
    ContainerStart(ContainerStartOp<'a>),
    /// An ng-container with no children.
    Container(ContainerOp<'a>),
    /// End an ng-container.
    ContainerEnd(ContainerEndOp<'a>),
    /// Disable bindings for descendants.
    DisableBindings(DisableBindingsOp<'a>),
    /// Re-enable bindings.
    EnableBindings(EnableBindingsOp<'a>),
    /// Create a conditional instruction.
    Conditional(ConditionalOp<'a>),
    /// Create a conditional branch instruction.
    ConditionalBranch(ConditionalBranchCreateOp<'a>),
    /// Create a control binding instruction.
    ControlCreate(ControlCreateOp<'a>),
    /// Render a text node.
    Text(TextOp<'a>),
    /// Declare an event listener.
    Listener(ListenerOp<'a>),
    /// Two-way listener (for two-way bindings).
    TwoWayListener(TwoWayListenerOp<'a>),
    /// Animation string binding.
    AnimationString(AnimationStringOp<'a>),
    /// Animation binding (converted from AnimationBinding, emits syntheticHostProperty).
    Animation(AnimationOp<'a>),
    /// Animation listener.
    AnimationListener(AnimationListenerOp<'a>),
    /// Create a pipe instance.
    Pipe(PipeOp<'a>),
    /// Configure a @defer block.
    Defer(DeferOp<'a>),
    /// Defer trigger.
    DeferOn(DeferOnOp<'a>),
    /// An i18n message.
    I18nMessage(I18nMessageOp<'a>),
    /// Change namespace.
    Namespace(NamespaceOp<'a>),
    /// Configure content projection.
    ProjectionDef(ProjectionDefOp<'a>),
    /// Create a content projection slot.
    Projection(ProjectionOp<'a>),
    /// Create a repeater instruction.
    RepeaterCreate(RepeaterCreateOp<'a>),
    /// Initialize a @let slot.
    DeclareLet(DeclareLetOp<'a>),
    /// Start an i18n block.
    I18nStart(I18nStartOp<'a>),
    /// Self-closing i18n on an element.
    I18n(I18nOp<'a>),
    /// End an i18n block.
    I18nEnd(I18nEndOp<'a>),
    /// Create an ICU expression.
    IcuStart(IcuStartOp<'a>),
    /// End an ICU expression.
    IcuEnd(IcuEndOp<'a>),
    /// ICU placeholder (replaces Text within ICU).
    IcuPlaceholder(IcuPlaceholderOp<'a>),
    /// An i18n context.
    I18nContext(I18nContextOp<'a>),
    /// I18n attributes on an element.
    I18nAttributes(I18nAttributesOp<'a>),
    /// Semantic variable declaration.
    Variable(VariableOp<'a>),
    /// Extracted attribute for consts array.
    ExtractedAttribute(ExtractedAttributeOp<'a>),
    /// Source location for debugging.
    SourceLocation(SourceLocationOp<'a>),
    /// An output AST statement (for listener handlers in create context).
    Statement(CreateStatementOp<'a>),
}

impl<'a> Op for CreateOp<'a> {
    fn kind(&self) -> OpKind {
        match self {
            CreateOp::ListEnd(_) => OpKind::ListEnd,
            CreateOp::ElementStart(_) => OpKind::ElementStart,
            CreateOp::Element(_) => OpKind::Element,
            CreateOp::ElementEnd(_) => OpKind::ElementEnd,
            CreateOp::Template(_) => OpKind::Template,
            CreateOp::ContainerStart(_) => OpKind::ContainerStart,
            CreateOp::Container(_) => OpKind::Container,
            CreateOp::ContainerEnd(_) => OpKind::ContainerEnd,
            CreateOp::DisableBindings(_) => OpKind::DisableBindings,
            CreateOp::EnableBindings(_) => OpKind::EnableBindings,
            CreateOp::Conditional(_) => OpKind::ConditionalCreate,
            CreateOp::ConditionalBranch(_) => OpKind::ConditionalBranchCreate,
            CreateOp::ControlCreate(_) => OpKind::ControlCreate,
            CreateOp::Text(_) => OpKind::Text,
            CreateOp::Listener(_) => OpKind::Listener,
            CreateOp::TwoWayListener(_) => OpKind::TwoWayListener,
            CreateOp::AnimationString(_) => OpKind::AnimationString,
            CreateOp::Animation(_) => OpKind::Animation,
            CreateOp::AnimationListener(_) => OpKind::AnimationListener,
            CreateOp::Pipe(_) => OpKind::Pipe,
            CreateOp::Defer(_) => OpKind::Defer,
            CreateOp::DeferOn(_) => OpKind::DeferOn,
            CreateOp::I18nMessage(_) => OpKind::I18nMessage,
            CreateOp::Namespace(_) => OpKind::Namespace,
            CreateOp::ProjectionDef(_) => OpKind::ProjectionDef,
            CreateOp::Projection(_) => OpKind::Projection,
            CreateOp::RepeaterCreate(_) => OpKind::RepeaterCreate,
            CreateOp::DeclareLet(_) => OpKind::DeclareLet,
            CreateOp::I18nStart(_) => OpKind::I18nStart,
            CreateOp::I18n(_) => OpKind::I18n,
            CreateOp::I18nEnd(_) => OpKind::I18nEnd,
            CreateOp::IcuStart(_) => OpKind::IcuStart,
            CreateOp::IcuEnd(_) => OpKind::IcuEnd,
            CreateOp::IcuPlaceholder(_) => OpKind::IcuPlaceholder,
            CreateOp::I18nContext(_) => OpKind::I18nContext,
            CreateOp::I18nAttributes(_) => OpKind::I18nAttributes,
            CreateOp::Variable(_) => OpKind::Variable,
            CreateOp::ExtractedAttribute(_) => OpKind::ExtractedAttribute,
            CreateOp::SourceLocation(_) => OpKind::SourceLocation,
            CreateOp::Statement(_) => OpKind::Statement,
        }
    }

    fn prev(&self) -> Option<NonNull<Self>> {
        match self {
            CreateOp::ListEnd(op) => op.base.prev,
            CreateOp::ElementStart(op) => op.base.prev,
            CreateOp::Element(op) => op.base.prev,
            CreateOp::ElementEnd(op) => op.base.prev,
            CreateOp::Template(op) => op.base.prev,
            CreateOp::ContainerStart(op) => op.base.prev,
            CreateOp::Container(op) => op.base.prev,
            CreateOp::ContainerEnd(op) => op.base.prev,
            CreateOp::DisableBindings(op) => op.base.prev,
            CreateOp::EnableBindings(op) => op.base.prev,
            CreateOp::Conditional(op) => op.base.prev,
            CreateOp::ConditionalBranch(op) => op.base.prev,
            CreateOp::ControlCreate(op) => op.base.prev,
            CreateOp::Text(op) => op.base.prev,
            CreateOp::Listener(op) => op.base.prev,
            CreateOp::TwoWayListener(op) => op.base.prev,
            CreateOp::AnimationString(op) => op.base.prev,
            CreateOp::Animation(op) => op.base.prev,
            CreateOp::AnimationListener(op) => op.base.prev,
            CreateOp::Pipe(op) => op.base.prev,
            CreateOp::Defer(op) => op.base.prev,
            CreateOp::DeferOn(op) => op.base.prev,
            CreateOp::I18nMessage(op) => op.base.prev,
            CreateOp::Namespace(op) => op.base.prev,
            CreateOp::ProjectionDef(op) => op.base.prev,
            CreateOp::Projection(op) => op.base.prev,
            CreateOp::RepeaterCreate(op) => op.base.prev,
            CreateOp::DeclareLet(op) => op.base.prev,
            CreateOp::I18nStart(op) => op.base.prev,
            CreateOp::I18n(op) => op.base.prev,
            CreateOp::I18nEnd(op) => op.base.prev,
            CreateOp::IcuStart(op) => op.base.prev,
            CreateOp::IcuEnd(op) => op.base.prev,
            CreateOp::IcuPlaceholder(op) => op.base.prev,
            CreateOp::I18nContext(op) => op.base.prev,
            CreateOp::I18nAttributes(op) => op.base.prev,
            CreateOp::Variable(op) => op.base.prev,
            CreateOp::ExtractedAttribute(op) => op.base.prev,
            CreateOp::SourceLocation(op) => op.base.prev,
            CreateOp::Statement(op) => op.base.prev,
        }
    }

    fn next(&self) -> Option<NonNull<Self>> {
        match self {
            CreateOp::ListEnd(op) => op.base.next,
            CreateOp::ElementStart(op) => op.base.next,
            CreateOp::Element(op) => op.base.next,
            CreateOp::ElementEnd(op) => op.base.next,
            CreateOp::Template(op) => op.base.next,
            CreateOp::ContainerStart(op) => op.base.next,
            CreateOp::Container(op) => op.base.next,
            CreateOp::ContainerEnd(op) => op.base.next,
            CreateOp::DisableBindings(op) => op.base.next,
            CreateOp::EnableBindings(op) => op.base.next,
            CreateOp::Conditional(op) => op.base.next,
            CreateOp::ConditionalBranch(op) => op.base.next,
            CreateOp::ControlCreate(op) => op.base.next,
            CreateOp::Text(op) => op.base.next,
            CreateOp::Listener(op) => op.base.next,
            CreateOp::TwoWayListener(op) => op.base.next,
            CreateOp::AnimationString(op) => op.base.next,
            CreateOp::Animation(op) => op.base.next,
            CreateOp::AnimationListener(op) => op.base.next,
            CreateOp::Pipe(op) => op.base.next,
            CreateOp::Defer(op) => op.base.next,
            CreateOp::DeferOn(op) => op.base.next,
            CreateOp::I18nMessage(op) => op.base.next,
            CreateOp::Namespace(op) => op.base.next,
            CreateOp::ProjectionDef(op) => op.base.next,
            CreateOp::Projection(op) => op.base.next,
            CreateOp::RepeaterCreate(op) => op.base.next,
            CreateOp::DeclareLet(op) => op.base.next,
            CreateOp::I18nStart(op) => op.base.next,
            CreateOp::I18n(op) => op.base.next,
            CreateOp::I18nEnd(op) => op.base.next,
            CreateOp::IcuStart(op) => op.base.next,
            CreateOp::IcuEnd(op) => op.base.next,
            CreateOp::IcuPlaceholder(op) => op.base.next,
            CreateOp::I18nContext(op) => op.base.next,
            CreateOp::I18nAttributes(op) => op.base.next,
            CreateOp::Variable(op) => op.base.next,
            CreateOp::ExtractedAttribute(op) => op.base.next,
            CreateOp::SourceLocation(op) => op.base.next,
            CreateOp::Statement(op) => op.base.next,
        }
    }

    fn set_prev(&mut self, prev: Option<NonNull<Self>>) {
        match self {
            CreateOp::ListEnd(op) => op.base.prev = prev,
            CreateOp::ElementStart(op) => op.base.prev = prev,
            CreateOp::Element(op) => op.base.prev = prev,
            CreateOp::ElementEnd(op) => op.base.prev = prev,
            CreateOp::Template(op) => op.base.prev = prev,
            CreateOp::ContainerStart(op) => op.base.prev = prev,
            CreateOp::Container(op) => op.base.prev = prev,
            CreateOp::ContainerEnd(op) => op.base.prev = prev,
            CreateOp::DisableBindings(op) => op.base.prev = prev,
            CreateOp::EnableBindings(op) => op.base.prev = prev,
            CreateOp::Conditional(op) => op.base.prev = prev,
            CreateOp::ConditionalBranch(op) => op.base.prev = prev,
            CreateOp::ControlCreate(op) => op.base.prev = prev,
            CreateOp::Text(op) => op.base.prev = prev,
            CreateOp::Listener(op) => op.base.prev = prev,
            CreateOp::TwoWayListener(op) => op.base.prev = prev,
            CreateOp::AnimationString(op) => op.base.prev = prev,
            CreateOp::Animation(op) => op.base.prev = prev,
            CreateOp::AnimationListener(op) => op.base.prev = prev,
            CreateOp::Pipe(op) => op.base.prev = prev,
            CreateOp::Defer(op) => op.base.prev = prev,
            CreateOp::DeferOn(op) => op.base.prev = prev,
            CreateOp::I18nMessage(op) => op.base.prev = prev,
            CreateOp::Namespace(op) => op.base.prev = prev,
            CreateOp::ProjectionDef(op) => op.base.prev = prev,
            CreateOp::Projection(op) => op.base.prev = prev,
            CreateOp::RepeaterCreate(op) => op.base.prev = prev,
            CreateOp::DeclareLet(op) => op.base.prev = prev,
            CreateOp::I18nStart(op) => op.base.prev = prev,
            CreateOp::I18n(op) => op.base.prev = prev,
            CreateOp::I18nEnd(op) => op.base.prev = prev,
            CreateOp::IcuStart(op) => op.base.prev = prev,
            CreateOp::IcuEnd(op) => op.base.prev = prev,
            CreateOp::IcuPlaceholder(op) => op.base.prev = prev,
            CreateOp::I18nContext(op) => op.base.prev = prev,
            CreateOp::I18nAttributes(op) => op.base.prev = prev,
            CreateOp::Variable(op) => op.base.prev = prev,
            CreateOp::ExtractedAttribute(op) => op.base.prev = prev,
            CreateOp::SourceLocation(op) => op.base.prev = prev,
            CreateOp::Statement(op) => op.base.prev = prev,
        }
    }

    fn set_next(&mut self, next: Option<NonNull<Self>>) {
        match self {
            CreateOp::ListEnd(op) => op.base.next = next,
            CreateOp::ElementStart(op) => op.base.next = next,
            CreateOp::Element(op) => op.base.next = next,
            CreateOp::ElementEnd(op) => op.base.next = next,
            CreateOp::Template(op) => op.base.next = next,
            CreateOp::ContainerStart(op) => op.base.next = next,
            CreateOp::Container(op) => op.base.next = next,
            CreateOp::ContainerEnd(op) => op.base.next = next,
            CreateOp::DisableBindings(op) => op.base.next = next,
            CreateOp::EnableBindings(op) => op.base.next = next,
            CreateOp::Conditional(op) => op.base.next = next,
            CreateOp::ConditionalBranch(op) => op.base.next = next,
            CreateOp::ControlCreate(op) => op.base.next = next,
            CreateOp::Text(op) => op.base.next = next,
            CreateOp::Listener(op) => op.base.next = next,
            CreateOp::TwoWayListener(op) => op.base.next = next,
            CreateOp::AnimationString(op) => op.base.next = next,
            CreateOp::Animation(op) => op.base.next = next,
            CreateOp::AnimationListener(op) => op.base.next = next,
            CreateOp::Pipe(op) => op.base.next = next,
            CreateOp::Defer(op) => op.base.next = next,
            CreateOp::DeferOn(op) => op.base.next = next,
            CreateOp::I18nMessage(op) => op.base.next = next,
            CreateOp::Namespace(op) => op.base.next = next,
            CreateOp::ProjectionDef(op) => op.base.next = next,
            CreateOp::Projection(op) => op.base.next = next,
            CreateOp::RepeaterCreate(op) => op.base.next = next,
            CreateOp::DeclareLet(op) => op.base.next = next,
            CreateOp::I18nStart(op) => op.base.next = next,
            CreateOp::I18n(op) => op.base.next = next,
            CreateOp::I18nEnd(op) => op.base.next = next,
            CreateOp::IcuStart(op) => op.base.next = next,
            CreateOp::IcuEnd(op) => op.base.next = next,
            CreateOp::IcuPlaceholder(op) => op.base.next = next,
            CreateOp::I18nContext(op) => op.base.next = next,
            CreateOp::I18nAttributes(op) => op.base.next = next,
            CreateOp::Variable(op) => op.base.next = next,
            CreateOp::ExtractedAttribute(op) => op.base.next = next,
            CreateOp::SourceLocation(op) => op.base.next = next,
            CreateOp::Statement(op) => op.base.next = next,
        }
    }
}

// ============================================================================
// Update Operations
// ============================================================================

/// Base fields common to all update operations.
#[derive(Debug)]
pub struct UpdateOpBase<'a> {
    /// Link to previous operation.
    pub prev: Option<NonNull<UpdateOp<'a>>>,
    /// Link to next operation.
    pub next: Option<NonNull<UpdateOp<'a>>>,
    /// Source span for debugging.
    pub source_span: Option<Span>,
}

impl<'a> Default for UpdateOpBase<'a> {
    fn default() -> Self {
        Self { prev: None, next: None, source_span: None }
    }
}

/// Update-time operation.
#[derive(Debug)]
pub enum UpdateOp<'a> {
    /// List end marker.
    ListEnd(UpdateListEndOp<'a>),
    /// Interpolate text into a text node.
    InterpolateText(InterpolateTextOp<'a>),
    /// Bind to an element property.
    Property(PropertyOp<'a>),
    /// Bind to a style property.
    StyleProp(StylePropOp<'a>),
    /// Bind to a class property.
    ClassProp(ClassPropOp<'a>),
    /// Bind to styles.
    StyleMap(StyleMapOp<'a>),
    /// Bind to classes.
    ClassMap(ClassMapOp<'a>),
    /// Advance the runtime's implicit slot context.
    Advance(AdvanceOp<'a>),
    /// Associate an attribute with an element.
    Attribute(AttributeOp<'a>),
    /// DOM property binding.
    DomProperty(DomPropertyOp<'a>),
    /// Update a repeater.
    Repeater(RepeaterOp<'a>),
    /// Two-way property binding.
    TwoWayProperty(TwoWayPropertyOp<'a>),
    /// Store @let value.
    StoreLet(StoreLetOp<'a>),
    /// Update conditional.
    Conditional(ConditionalUpdateOp<'a>),
    /// An i18n expression.
    I18nExpression(I18nExpressionOp<'a>),
    /// Apply i18n expressions.
    I18nApply(I18nApplyOp<'a>),
    /// Intermediate binding.
    Binding(BindingOp<'a>),
    /// Animation binding.
    AnimationBinding(AnimationBindingOp<'a>),
    /// Semantic variable declaration.
    Variable(UpdateVariableOp<'a>),
    /// Control binding.
    Control(ControlOp<'a>),
    /// An output AST statement (for listener handlers).
    Statement(StatementOp<'a>),
    /// Defer with condition (evaluated on every change detection).
    DeferWhen(DeferWhenOp<'a>),
}

impl<'a> Op for UpdateOp<'a> {
    fn kind(&self) -> OpKind {
        match self {
            UpdateOp::ListEnd(_) => OpKind::ListEnd,
            UpdateOp::InterpolateText(_) => OpKind::InterpolateText,
            UpdateOp::Property(_) => OpKind::Property,
            UpdateOp::StyleProp(_) => OpKind::StyleProp,
            UpdateOp::ClassProp(_) => OpKind::ClassProp,
            UpdateOp::StyleMap(_) => OpKind::StyleMap,
            UpdateOp::ClassMap(_) => OpKind::ClassMap,
            UpdateOp::Advance(_) => OpKind::Advance,
            UpdateOp::Attribute(_) => OpKind::Attribute,
            UpdateOp::DomProperty(_) => OpKind::DomProperty,
            UpdateOp::Repeater(_) => OpKind::Repeater,
            UpdateOp::TwoWayProperty(_) => OpKind::TwoWayProperty,
            UpdateOp::StoreLet(_) => OpKind::StoreLet,
            UpdateOp::Conditional(_) => OpKind::Conditional,
            UpdateOp::I18nExpression(_) => OpKind::I18nExpression,
            UpdateOp::I18nApply(_) => OpKind::I18nApply,
            UpdateOp::Binding(_) => OpKind::Binding,
            UpdateOp::AnimationBinding(_) => OpKind::AnimationBinding,
            UpdateOp::Variable(_) => OpKind::Variable,
            UpdateOp::Control(_) => OpKind::Control,
            UpdateOp::Statement(_) => OpKind::Statement,
            UpdateOp::DeferWhen(_) => OpKind::DeferWhen,
        }
    }

    fn prev(&self) -> Option<NonNull<Self>> {
        match self {
            UpdateOp::ListEnd(op) => op.base.prev,
            UpdateOp::InterpolateText(op) => op.base.prev,
            UpdateOp::Property(op) => op.base.prev,
            UpdateOp::StyleProp(op) => op.base.prev,
            UpdateOp::ClassProp(op) => op.base.prev,
            UpdateOp::StyleMap(op) => op.base.prev,
            UpdateOp::ClassMap(op) => op.base.prev,
            UpdateOp::Advance(op) => op.base.prev,
            UpdateOp::Attribute(op) => op.base.prev,
            UpdateOp::DomProperty(op) => op.base.prev,
            UpdateOp::Repeater(op) => op.base.prev,
            UpdateOp::TwoWayProperty(op) => op.base.prev,
            UpdateOp::StoreLet(op) => op.base.prev,
            UpdateOp::Conditional(op) => op.base.prev,
            UpdateOp::I18nExpression(op) => op.base.prev,
            UpdateOp::I18nApply(op) => op.base.prev,
            UpdateOp::Binding(op) => op.base.prev,
            UpdateOp::AnimationBinding(op) => op.base.prev,
            UpdateOp::Variable(op) => op.base.prev,
            UpdateOp::Control(op) => op.base.prev,
            UpdateOp::Statement(op) => op.base.prev,
            UpdateOp::DeferWhen(op) => op.base.prev,
        }
    }

    fn next(&self) -> Option<NonNull<Self>> {
        match self {
            UpdateOp::ListEnd(op) => op.base.next,
            UpdateOp::InterpolateText(op) => op.base.next,
            UpdateOp::Property(op) => op.base.next,
            UpdateOp::StyleProp(op) => op.base.next,
            UpdateOp::ClassProp(op) => op.base.next,
            UpdateOp::StyleMap(op) => op.base.next,
            UpdateOp::ClassMap(op) => op.base.next,
            UpdateOp::Advance(op) => op.base.next,
            UpdateOp::Attribute(op) => op.base.next,
            UpdateOp::DomProperty(op) => op.base.next,
            UpdateOp::Repeater(op) => op.base.next,
            UpdateOp::TwoWayProperty(op) => op.base.next,
            UpdateOp::StoreLet(op) => op.base.next,
            UpdateOp::Conditional(op) => op.base.next,
            UpdateOp::I18nExpression(op) => op.base.next,
            UpdateOp::I18nApply(op) => op.base.next,
            UpdateOp::Binding(op) => op.base.next,
            UpdateOp::AnimationBinding(op) => op.base.next,
            UpdateOp::Variable(op) => op.base.next,
            UpdateOp::Control(op) => op.base.next,
            UpdateOp::Statement(op) => op.base.next,
            UpdateOp::DeferWhen(op) => op.base.next,
        }
    }

    fn set_prev(&mut self, prev: Option<NonNull<Self>>) {
        match self {
            UpdateOp::ListEnd(op) => op.base.prev = prev,
            UpdateOp::InterpolateText(op) => op.base.prev = prev,
            UpdateOp::Property(op) => op.base.prev = prev,
            UpdateOp::StyleProp(op) => op.base.prev = prev,
            UpdateOp::ClassProp(op) => op.base.prev = prev,
            UpdateOp::StyleMap(op) => op.base.prev = prev,
            UpdateOp::ClassMap(op) => op.base.prev = prev,
            UpdateOp::Advance(op) => op.base.prev = prev,
            UpdateOp::Attribute(op) => op.base.prev = prev,
            UpdateOp::DomProperty(op) => op.base.prev = prev,
            UpdateOp::Repeater(op) => op.base.prev = prev,
            UpdateOp::TwoWayProperty(op) => op.base.prev = prev,
            UpdateOp::StoreLet(op) => op.base.prev = prev,
            UpdateOp::Conditional(op) => op.base.prev = prev,
            UpdateOp::I18nExpression(op) => op.base.prev = prev,
            UpdateOp::I18nApply(op) => op.base.prev = prev,
            UpdateOp::Binding(op) => op.base.prev = prev,
            UpdateOp::AnimationBinding(op) => op.base.prev = prev,
            UpdateOp::Variable(op) => op.base.prev = prev,
            UpdateOp::Control(op) => op.base.prev = prev,
            UpdateOp::Statement(op) => op.base.prev = prev,
            UpdateOp::DeferWhen(op) => op.base.prev = prev,
        }
    }

    fn set_next(&mut self, next: Option<NonNull<Self>>) {
        match self {
            UpdateOp::ListEnd(op) => op.base.next = next,
            UpdateOp::InterpolateText(op) => op.base.next = next,
            UpdateOp::Property(op) => op.base.next = next,
            UpdateOp::StyleProp(op) => op.base.next = next,
            UpdateOp::ClassProp(op) => op.base.next = next,
            UpdateOp::StyleMap(op) => op.base.next = next,
            UpdateOp::ClassMap(op) => op.base.next = next,
            UpdateOp::Advance(op) => op.base.next = next,
            UpdateOp::Attribute(op) => op.base.next = next,
            UpdateOp::DomProperty(op) => op.base.next = next,
            UpdateOp::Repeater(op) => op.base.next = next,
            UpdateOp::TwoWayProperty(op) => op.base.next = next,
            UpdateOp::StoreLet(op) => op.base.next = next,
            UpdateOp::Conditional(op) => op.base.next = next,
            UpdateOp::I18nExpression(op) => op.base.next = next,
            UpdateOp::I18nApply(op) => op.base.next = next,
            UpdateOp::Binding(op) => op.base.next = next,
            UpdateOp::AnimationBinding(op) => op.base.next = next,
            UpdateOp::Variable(op) => op.base.next = next,
            UpdateOp::Control(op) => op.base.next = next,
            UpdateOp::Statement(op) => op.base.next = next,
            UpdateOp::DeferWhen(op) => op.base.next = next,
        }
    }
}

// ============================================================================
// Create Operation Structures
// ============================================================================

/// List end marker for create operations.
#[derive(Debug)]
pub struct ListEndOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
}

/// Start rendering an element.
#[derive(Debug)]
pub struct ElementStartOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Element tag name.
    pub tag: Atom<'a>,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Namespace.
    pub namespace: Namespace,
    /// Attribute namespace (e.g., "xlink" for SVG).
    pub attribute_namespace: Option<Atom<'a>>,
    /// Local references attached to this element.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Index into the consts array for local refs.
    /// Set by local_refs phase.
    pub local_refs_index: Option<u32>,
    /// Non-bindable element.
    pub non_bindable: bool,
    /// I18n placeholder data (start_name and close_name for paired tags).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Index into the consts array for element attributes.
    pub attributes: Option<u32>,
}

/// Render an element with no children.
#[derive(Debug)]
pub struct ElementOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Element tag name.
    pub tag: Atom<'a>,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Namespace.
    pub namespace: Namespace,
    /// Attribute namespace.
    pub attribute_namespace: Option<Atom<'a>>,
    /// Local references.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Index into the consts array for local refs.
    /// Set by local_refs phase.
    pub local_refs_index: Option<u32>,
    /// Non-bindable element.
    pub non_bindable: bool,
    /// I18n placeholder data (start_name and close_name for paired tags).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Index into the consts array for element attributes.
    pub attributes: Option<u32>,
}

/// End rendering an element.
#[derive(Debug)]
pub struct ElementEndOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference to matching ElementStart.
    pub xref: XrefId,
}

/// Create a template (embedded view).
#[derive(Debug)]
pub struct TemplateOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Embedded view xref.
    pub embedded_view: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// HTML tag name for this template element.
    /// Used for content projection matching (e.g., `<ng-template>` vs `<div *ngIf="...">`).
    pub tag: Option<Atom<'a>>,
    /// Namespace.
    pub namespace: Namespace,
    /// Template kind.
    pub template_kind: TemplateKind,
    /// Function name suffix.
    pub fn_name_suffix: Option<Atom<'a>>,
    /// Block of the template.
    pub block: Option<Atom<'a>>,
    /// Decl count for structural directive compatibility.
    pub decl_count: Option<u32>,
    /// The number of binding variable slots used by this template.
    /// Set by var_counting phase.
    pub vars: Option<u32>,
    /// Index into the consts array for template attributes.
    /// Set by attribute extraction phase.
    pub attributes: Option<u32>,
    /// Local references on this template.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Index into the consts array for local refs.
    /// Set by local_refs extraction phase.
    pub local_refs_index: Option<u32>,
    /// I18n placeholder data (start_name and close_name for paired tags).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
}

/// Start an ng-container.
#[derive(Debug)]
pub struct ContainerStartOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Index into the consts array for attributes.
    /// Set by const_collection phase.
    pub attributes: Option<u32>,
    /// Index into the consts array for local refs.
    /// Set by local_refs extraction phase.
    pub local_refs_index: Option<u32>,
    /// Local references.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Non-bindable container.
    pub non_bindable: bool,
    /// I18n placeholder data (start_name and close_name for paired tags).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
}

/// An ng-container with no children.
#[derive(Debug)]
pub struct ContainerOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Index into the consts array for attributes.
    /// Set by const_collection phase.
    pub attributes: Option<u32>,
    /// Index into the consts array for local refs.
    /// Set by local_refs extraction phase.
    pub local_refs_index: Option<u32>,
    /// Local references.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Non-bindable container.
    pub non_bindable: bool,
    /// I18n placeholder data (start_name and close_name for paired tags).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
}

/// End an ng-container.
#[derive(Debug)]
pub struct ContainerEndOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference to matching ContainerStart.
    pub xref: XrefId,
}

/// Disable bindings for descendants.
#[derive(Debug)]
pub struct DisableBindingsOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference to element.
    pub xref: XrefId,
}

/// Re-enable bindings.
#[derive(Debug)]
pub struct EnableBindingsOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference to element.
    pub xref: XrefId,
}

/// Create a conditional instruction (@if/@switch).
///
/// This represents the first branch of a conditional block.
/// Subsequent branches are represented as `ConditionalBranchCreateOp`.
/// The actual conditional logic (test, conditions) is in the update op `ConditionalUpdateOp`.
///
/// Ported from Angular's `ConditionalCreateOp` in `create.ts`.
#[derive(Debug)]
pub struct ConditionalOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Namespace.
    pub namespace: Namespace,
    /// Template kind.
    pub template_kind: TemplateKind,
    /// Function name suffix.
    pub fn_name_suffix: Atom<'a>,
    /// HTML tag name (for content projection).
    pub tag: Option<Atom<'a>>,
    /// The number of declaration slots used by this conditional.
    /// Set by allocate_slots phase.
    pub decls: Option<u32>,
    /// The number of binding variable slots used by this conditional.
    /// Set by var_counting phase.
    pub vars: Option<u32>,
    /// Local references on this element.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Index into the consts array for local refs.
    /// Set by local_refs phase.
    pub local_refs_index: Option<u32>,
    /// I18n placeholder data (start_name and close_name for @if/@switch blocks).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Index into the consts array for element attributes.
    pub attributes: Option<u32>,
    /// Non-bindable element.
    pub non_bindable: bool,
}

/// Create a conditional branch instruction.
///
/// This is used for branches after the first one in an @if/@switch block.
/// The first branch is part of the parent `ConditionalOp`, but subsequent
/// branches (else if, else, case, default) are represented as separate
/// `ConditionalBranchCreateOp` operations.
#[derive(Debug)]
pub struct ConditionalBranchCreateOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Namespace.
    pub namespace: Namespace,
    /// Template kind.
    pub template_kind: TemplateKind,
    /// Function name suffix.
    pub fn_name_suffix: Atom<'a>,
    /// HTML tag name (for content projection).
    pub tag: Option<Atom<'a>>,
    /// The number of declaration slots used by this branch.
    /// Set by allocate_slots phase.
    pub decls: Option<u32>,
    /// The number of binding variable slots used by this branch.
    /// Set by var_counting phase.
    pub vars: Option<u32>,
    /// Local references on this branch.
    pub local_refs: Vec<'a, LocalRef<'a>>,
    /// Index into the consts array for local refs.
    /// Set by local_refs phase.
    pub local_refs_index: Option<u32>,
    /// I18n placeholder data (start_name and close_name for paired tags).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Index into the consts array for element attributes.
    pub attributes: Option<u32>,
    /// Non-bindable element.
    pub non_bindable: bool,
}

/// Create a control binding instruction.
///
/// This operation determines whether a `[control]` binding targets a specialized
/// control directive on a native or custom form control, and if so, adds event
/// listeners to synchronize the bound form field to the form control.
#[derive(Debug)]
pub struct ControlCreateOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
}

/// Render a text node.
#[derive(Debug)]
pub struct TextOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Static text value.
    pub initial_value: Atom<'a>,
    /// I18n placeholder.
    pub i18n_placeholder: Option<Atom<'a>>,
    /// ICU placeholder (for text inside ICU expressions).
    pub icu_placeholder: Option<Atom<'a>>,
}

/// Declare an event listener.
#[derive(Debug)]
pub struct ListenerOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Element or template reference.
    pub target: XrefId,
    /// Slot to target.
    pub target_slot: SlotId,
    /// Tag name of the element on which this listener is placed. Null for host bindings.
    pub tag: Option<Atom<'a>>,
    /// Whether this listener is from a host binding.
    pub host_listener: bool,
    /// Event name.
    pub name: Atom<'a>,
    /// Handler expression (the expression to execute on event).
    pub handler_expression: Option<Box<'a, crate::ir::expression::IrExpression<'a>>>,
    /// Handler operations (legacy, for complex handlers).
    pub handler_ops: Vec<'a, UpdateOp<'a>>,
    /// Function name.
    pub handler_fn_name: Option<Atom<'a>>,
    /// Consume functions.
    pub consume_fn_name: Option<Atom<'a>>,
    /// Whether this is an animation listener.
    pub is_animation_listener: bool,
    /// Animation phase.
    pub animation_phase: Option<AnimationKind>,
    /// Event target (window, document, body).
    pub event_target: Option<Atom<'a>>,
    /// Whether this listener uses $event.
    pub consumes_dollar_event: bool,
}

/// Create a pipe instance.
#[derive(Debug)]
pub struct PipeOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Pipe name.
    pub name: Atom<'a>,
    /// Number of arguments.
    pub num_args: u32,
}

/// Configure a @defer block.
#[derive(Debug)]
pub struct DeferOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Main content view xref.
    pub main_view: Option<XrefId>,
    /// Resolved main template slot.
    pub main_slot: Option<SlotId>,
    /// Placeholder view xref.
    pub placeholder_view: Option<XrefId>,
    /// Resolved placeholder template slot.
    pub placeholder_slot: Option<SlotId>,
    /// Loading view xref.
    pub loading_view: Option<XrefId>,
    /// Resolved loading template slot.
    pub loading_slot: Option<SlotId>,
    /// Error view xref.
    pub error_view: Option<XrefId>,
    /// Resolved error template slot.
    pub error_slot: Option<SlotId>,
    /// Placeholder minimum time.
    pub placeholder_minimum_time: Option<u32>,
    /// Loading minimum time.
    pub loading_minimum_time: Option<u32>,
    /// Loading after time.
    pub loading_after_time: Option<u32>,
    /// Placeholder config expression (wraps [minimumTime] as ConstCollectedExpr).
    /// Set by defer_configs phase; resolved to ConstReference by const collection phase.
    pub placeholder_config: Option<Box<'a, IrExpression<'a>>>,
    /// Loading config expression (wraps [minimumTime, afterTime] as ConstCollectedExpr).
    /// Set by defer_configs phase; resolved to ConstReference by const collection phase.
    pub loading_config: Option<Box<'a, IrExpression<'a>>>,
    /// Resolver function expression (after constant pool processing).
    /// This is the shared function reference created by resolve_defer_deps_fns.
    /// Corresponds to `resolverFn` in Angular TS.
    pub resolver_fn: Option<OutputExpression<'a>>,
    /// Own resolver function expression (before sharing).
    /// Set during ingestion from deferMeta.blocks (PerBlock) or allDeferrableDepsFn (PerComponent).
    /// Processed by resolve_defer_deps_fns to create resolver_fn.
    /// Corresponds to `ownResolverFn` in Angular TS.
    pub own_resolver_fn: Option<OutputExpression<'a>>,
    /// SSR unique ID.
    pub ssr_unique_id: Option<Atom<'a>>,
    /// Defer block flags (e.g., HasHydrateTriggers = 1).
    /// Corresponds to `flags` in Angular TS's DeferOp.
    pub flags: Option<u32>,
}

/// Defer trigger.
#[derive(Debug)]
pub struct DeferOnOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target defer block.
    pub defer: XrefId,
    /// Trigger kind.
    pub trigger: DeferTriggerKind,
    /// Trigger modifier.
    pub modifier: DeferOpModifierKind,
    /// Target element for viewport/interaction triggers.
    pub target_xref: Option<XrefId>,
    /// Target view containing the target element.
    pub target_view: Option<XrefId>,
    /// Target slot.
    pub target_slot: Option<SlotId>,
    /// Number of view traversal steps to reach target from trigger context.
    /// -1 means starting from placeholder, 0 means same view as defer owner.
    pub target_slot_view_steps: Option<i32>,
    /// Target name for local ref targeting.
    pub target_name: Option<Atom<'a>>,
    /// Timer delay.
    pub delay: Option<u32>,
    /// Viewport options (for viewport trigger).
    pub options: Option<Box<'a, IrExpression<'a>>>,
}

/// Defer with condition (update-time operation).
///
/// This operation evaluates a user-provided condition expression during
/// the update phase (change detection). When the condition becomes true,
/// it triggers the associated defer block.
///
/// Ported from Angular's `DeferWhenOp` in `ir/src/ops/update.ts`.
#[derive(Debug)]
pub struct DeferWhenOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target defer block.
    pub defer: XrefId,
    /// Condition expression.
    pub condition: Box<'a, IrExpression<'a>>,
    /// Trigger modifier.
    pub modifier: DeferOpModifierKind,
}

/// An i18n message.
#[derive(Debug)]
pub struct I18nMessageOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// I18n context.
    pub i18n_context: Option<XrefId>,
    /// The i18n block this message corresponds to.
    /// For regular i18n content, this is the I18nStart xref.
    /// For i18n attributes, this is None.
    pub i18n_block: Option<XrefId>,
    /// Message placeholder for ICU sub-messages.
    /// Only set for ICU placeholder messages (extracted from parent).
    pub message_placeholder: Option<Atom<'a>>,
    /// Message ID.
    pub message_id: Option<Atom<'a>>,
    /// Custom message ID.
    pub custom_id: Option<Atom<'a>>,
    /// Message meaning.
    pub meaning: Option<Atom<'a>>,
    /// Message description.
    pub description: Option<Atom<'a>>,
    /// The serialized message string for goog.getMsg and $localize.
    /// Contains the message text with placeholder markers like "{$interpolation}".
    pub message_string: Option<Atom<'a>>,
    /// Whether message needs postprocessing (has params with multiple values).
    pub needs_postprocessing: bool,
    /// Sub-messages.
    pub sub_messages: Vec<'a, XrefId>,
}

/// Change namespace.
#[derive(Debug)]
pub struct NamespaceOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Active namespace.
    pub active: Namespace,
}

/// Configure content projection.
///
/// Corresponds to the ɵɵprojectionDef() instruction.
/// The `def` field contains the R3 format selector array expression,
/// or None for the default single-wildcard case.
#[derive(Debug)]
pub struct ProjectionDefOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// The projection def expression in R3 format.
    /// None means default single-wildcard projection (no argument to projectionDef).
    pub def: Option<crate::output::ast::OutputExpression<'a>>,
}

/// Create a content projection slot.
#[derive(Debug)]
pub struct ProjectionOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Projection slot index.
    pub projection_slot_index: u32,
    /// I18n placeholder data (start_name and close_name for ng-content).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Selector attribute.
    pub selector: Option<Atom<'a>>,
    /// Fallback template.
    pub fallback: Option<XrefId>,
    /// I18n placeholder data for fallback view.
    pub fallback_i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Attributes array expression (set by const_collection phase).
    /// For ng-content, this contains the serialized attributes array directly,
    /// unlike elements which use a const index.
    pub attributes: Option<crate::output::ast::OutputExpression<'a>>,
}

/// Create a repeater instruction (@for).
#[derive(Debug)]
pub struct RepeaterCreateOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Body view xref.
    pub body_view: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Track function.
    pub track: Box<'a, IrExpression<'a>>,
    /// Track function name.
    pub track_fn_name: Option<Atom<'a>>,
    /// Some kinds of expressions (e.g. safe reads or nullish coalescing) require additional ops
    /// in order to work. This list keeps track of those ops, if they're necessary.
    /// Set by track_fn_optimization phase when the track expression cannot be optimized.
    pub track_by_ops: Option<Vec<'a, UpdateOp<'a>>>,
    /// Uses $index in track.
    pub uses_component_instance: bool,
    /// Empty view xref.
    pub empty_view: Option<XrefId>,
    /// Empty view slot.
    pub empty_slot: Option<SlotId>,
    /// Empty declaration count.
    pub empty_decl_count: Option<u32>,
    /// Empty variable count.
    /// Set by var_counting phase.
    pub empty_var_count: Option<u32>,
    /// The number of declaration slots used by this repeater's template.
    /// Set by allocate_slots phase.
    pub decls: Option<u32>,
    /// The number of binding variable slots used by this repeater.
    /// Set by var_counting phase.
    pub vars: Option<u32>,
    /// Var names for template.
    pub var_names: RepeaterVarNames<'a>,
    /// HTML tag name (for content projection).
    pub tag: Option<Atom<'a>>,
    /// Const index for attributes (for content projection).
    /// Set by const_collection phase.
    pub attributes: Option<u32>,
    /// HTML tag name for empty view (for content projection).
    pub empty_tag: Option<Atom<'a>>,
    /// Const index for empty view attributes (for content projection).
    /// Set by const_collection phase.
    pub empty_attributes: Option<u32>,
    /// I18n placeholder data (start_name and close_name for @for block).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// I18n placeholder data for @empty view.
    pub empty_i18n_placeholder: Option<I18nPlaceholder<'a>>,
}

/// Variable names for repeater template.
#[derive(Debug)]
pub struct RepeaterVarNames<'a> {
    /// Alias for the item ($implicit).
    pub item: Option<Atom<'a>>,
    /// Alias for $count.
    pub count: Option<Atom<'a>>,
    /// Alias for $index.
    pub index: Option<Atom<'a>>,
    /// Alias for $first.
    pub first: Option<Atom<'a>>,
    /// Alias for $last.
    pub last: Option<Atom<'a>>,
    /// Alias for $even.
    pub even: Option<Atom<'a>>,
    /// Alias for $odd.
    pub odd: Option<Atom<'a>>,
}

/// Initialize a @let slot.
#[derive(Debug)]
pub struct DeclareLetOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// Variable name.
    pub name: Atom<'a>,
}

/// Start an i18n block.
#[derive(Debug)]
pub struct I18nStartOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// I18n context.
    pub context: Option<XrefId>,
    /// Message instance ID for metadata lookup.
    pub message: Option<u32>,
    /// I18n placeholder data (start_name and close_name for i18n blocks).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Sub-template index for nested templates inside i18n blocks.
    /// None for root-level i18n blocks.
    pub sub_template_index: Option<u32>,
    /// Root i18n block reference (for nested i18n blocks).
    pub root: Option<XrefId>,
    /// Index into the consts array for the i18n message.
    /// Set by the i18n_const_collection phase.
    pub message_index: Option<u32>,
}

/// Self-closing i18n on an element.
#[derive(Debug)]
pub struct I18nOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Assigned slot.
    pub slot: Option<SlotId>,
    /// I18n context.
    pub context: Option<XrefId>,
    /// Message instance ID for metadata lookup.
    pub message: Option<u32>,
    /// I18n placeholder data (start_name and close_name for i18n blocks).
    pub i18n_placeholder: Option<I18nPlaceholder<'a>>,
    /// Sub-template index for nested templates inside i18n blocks.
    /// None for root-level i18n blocks.
    pub sub_template_index: Option<u32>,
    /// Root i18n block reference (for nested i18n blocks).
    pub root: Option<XrefId>,
    /// Index into the consts array for the i18n message.
    /// Set by the i18n_const_collection phase.
    pub message_index: Option<u32>,
}

/// End an i18n block.
#[derive(Debug)]
pub struct I18nEndOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference to matching I18nStart.
    pub xref: XrefId,
}

/// Create an ICU expression.
#[derive(Debug)]
pub struct IcuStartOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// I18n context.
    pub context: Option<XrefId>,
    /// Message instance ID for metadata lookup.
    pub message: Option<u32>,
    /// ICU placeholder.
    pub icu_placeholder: Option<Atom<'a>>,
}

/// End an ICU expression.
#[derive(Debug)]
pub struct IcuEndOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference to matching IcuStart.
    pub xref: XrefId,
}

/// An i18n context.
#[derive(Debug)]
pub struct I18nContextOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Context kind.
    pub context_kind: I18nContextKind,
    /// The i18n block this context belongs to (for RootI18n and Icu kinds).
    pub i18n_block: Option<XrefId>,
    /// Params map for recording i18n placeholder values.
    /// Maps placeholder names to lists of param values.
    pub params: oxc_allocator::HashMap<
        'a,
        Atom<'a>,
        oxc_allocator::Vec<'a, super::i18n_params::I18nParamValue>,
    >,
    /// Post-processing params map for ICU expressions.
    /// These are processed after the main params during message generation.
    pub postprocessing_params: oxc_allocator::HashMap<
        'a,
        Atom<'a>,
        oxc_allocator::Vec<'a, super::i18n_params::I18nParamValue>,
    >,
    /// ICU placeholder literals map.
    /// Maps ICU placeholder names to their formatted string values.
    /// These are string literals like "Hello ${�0�}!" generated from IcuPlaceholderOp.
    pub icu_placeholder_literals: oxc_allocator::HashMap<'a, Atom<'a>, Atom<'a>>,
    /// Message instance ID reference (for metadata lookup).
    ///
    /// Stores the i18n message's instance_id (not an XrefId) to look up metadata
    /// in the job's i18n_message_metadata map.
    pub message: Option<u32>,
}

/// I18n attributes on an element.
#[derive(Debug)]
pub struct I18nAttributesOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Handle (computed during slot allocation).
    pub handle: I18nSlotHandle,
    /// Target element.
    pub target: XrefId,
    /// Attribute configs (legacy - not actively used, kept for compatibility).
    pub configs: Vec<'a, I18nAttributeConfig<'a>>,
    /// I18nAttributes instructions correspond to a const array with configuration information.
    /// This field is populated by `i18n_const_collection` phase and used in reify.
    pub i18n_attributes_config: Option<u32>,
}

/// I18n attribute configuration.
#[derive(Debug)]
pub struct I18nAttributeConfig<'a> {
    /// Attribute name.
    pub name: Atom<'a>,
    /// I18n message.
    pub message: XrefId,
}

/// Semantic variable declaration.
#[derive(Debug)]
pub struct VariableOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Variable kind.
    pub kind: SemanticVariableKind,
    /// Variable name.
    pub name: Atom<'a>,
    /// Initializer expression.
    pub initializer: Box<'a, IrExpression<'a>>,
    /// Variable flags.
    pub flags: VariableFlags,
    /// View XrefId for Context and SavedView variables.
    /// For Context: the view whose context is being stored.
    /// For SavedView: the view being saved.
    pub view: Option<XrefId>,
    /// Whether this variable was declared locally within the same view (e.g., @let declarations).
    /// Local variables take precedence during name resolution over non-local variables.
    pub local: bool,
}

/// Extracted attribute for consts array.
#[derive(Debug)]
pub struct ExtractedAttributeOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Binding kind.
    pub binding_kind: BindingKind,
    /// Namespace.
    pub namespace: Option<Atom<'a>>,
    /// Attribute name.
    pub name: Atom<'a>,
    /// Attribute value.
    pub value: Option<Box<'a, IrExpression<'a>>>,
    /// Security context.
    pub security_context: SecurityContext,
    /// Whether expression is truthy.
    pub truthy_expression: bool,
    /// i18n message instance ID (for i18n attributes).
    ///
    /// This stores the i18n message's instance_id rather than an XrefId to avoid
    /// allocating xrefs during ingest. Angular TS stores a direct object reference
    /// to the i18n.Message; we use the instance_id as a dedup key instead.
    /// The actual xref for the i18n context is allocated later in create_i18n_contexts.
    pub i18n_message: Option<u32>,
    /// i18n context.
    pub i18n_context: Option<XrefId>,
    /// Trusted value function for security-sensitive constant attributes.
    pub trusted_value_fn: Option<Atom<'a>>,
}

/// Source location for debugging.
#[derive(Debug)]
pub struct SourceLocationOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Template URL.
    pub template_url: Atom<'a>,
    /// Line number.
    pub line: u32,
    /// Column number.
    pub column: u32,
}

/// An output AST statement for create operations.
///
/// This wraps an OutputStatement (ExpressionStatement, ReturnStatement, etc.)
/// and is used in create-time contexts. Often `StatementOp`s are the final
/// result of IR processing.
///
/// Ported from Angular's `StatementOp<CreateOp>` in `ir/src/ops/shared.ts`.
#[derive(Debug)]
pub struct CreateStatementOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// The output statement.
    pub statement: crate::output::ast::OutputStatement<'a>,
}

// ============================================================================
// Update Operation Structures
// ============================================================================

/// List end marker for update operations.
#[derive(Debug)]
pub struct UpdateListEndOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
}

/// Interpolate text into a text node.
#[derive(Debug)]
pub struct InterpolateTextOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target text node.
    pub target: XrefId,
    /// Interpolation expression.
    pub interpolation: Box<'a, IrExpression<'a>>,
    /// I18n placeholder.
    pub i18n_placeholder: Option<Atom<'a>>,
}

/// Bind to an element property.
#[derive(Debug)]
pub struct PropertyOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Property name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Whether this is a host binding.
    pub is_host: bool,
    /// Security context.
    pub security_context: SecurityContext,
    /// Sanitizer function.
    pub sanitizer: Option<Atom<'a>>,
    /// Whether property should be updated structurally.
    pub is_structural: bool,
    /// I18n context.
    pub i18n_context: Option<XrefId>,
    /// I18n message instance ID.
    ///
    /// Stores the i18n message's instance_id for dedup, not an XrefId.
    pub i18n_message: Option<u32>,
    /// Binding kind (for DomOnly mode and animation handling).
    pub binding_kind: BindingKind,
}

/// Bind to a style property.
#[derive(Debug)]
pub struct StylePropOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Style name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Unit suffix.
    pub unit: Option<Atom<'a>>,
}

/// Bind to a class property.
#[derive(Debug)]
pub struct ClassPropOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Class name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
}

/// Bind to styles.
#[derive(Debug)]
pub struct StyleMapOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
}

/// Bind to classes.
#[derive(Debug)]
pub struct ClassMapOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
}

/// Advance the runtime's implicit slot context.
#[derive(Debug)]
pub struct AdvanceOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Delta to advance.
    pub delta: u32,
    /// Target slot.
    pub slot: SlotId,
}

/// Associate an attribute with an element.
#[derive(Debug)]
pub struct AttributeOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Attribute name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Namespace.
    pub namespace: Option<Atom<'a>>,
    /// Security context.
    pub security_context: SecurityContext,
    /// Sanitizer function.
    pub sanitizer: Option<Atom<'a>>,
    /// I18n context.
    pub i18n_context: Option<XrefId>,
    /// I18n message instance ID.
    ///
    /// Stores the i18n message's instance_id for dedup, not an XrefId.
    pub i18n_message: Option<u32>,
    /// Whether this is a text attribute (static attribute from template).
    ///
    /// Text attributes are extractable to the consts array and don't need
    /// runtime updates. This is used by attribute_extraction to determine
    /// if the attribute should be extracted.
    pub is_text_attribute: bool,
    /// Whether this attribute is from a structural directive template
    /// (e.g., `ngFor` from `*ngFor="let item of items"`).
    ///
    /// Structural template attributes should be extracted with `BindingKind::Template`
    /// instead of `BindingKind::Attribute` for proper directive matching.
    pub is_structural_template_attribute: bool,
}

/// DOM property binding.
#[derive(Debug)]
pub struct DomPropertyOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Property name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Whether this is a host binding.
    pub is_host: bool,
    /// Security context.
    pub security_context: SecurityContext,
    /// Sanitizer function.
    pub sanitizer: Option<Atom<'a>>,
    /// Binding kind (for animation handling).
    pub binding_kind: BindingKind,
}

/// Update a repeater.
#[derive(Debug)]
pub struct RepeaterOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target repeater.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotId,
    /// Collection expression.
    pub collection: Box<'a, IrExpression<'a>>,
}

/// Two-way property binding.
#[derive(Debug)]
pub struct TwoWayPropertyOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Property name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Security context.
    pub security_context: SecurityContext,
    /// Sanitizer function.
    pub sanitizer: Option<Atom<'a>>,
}

/// Two-way listener (CREATE op).
#[derive(Debug)]
pub struct TwoWayListenerOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotId,
    /// Tag name of the element on which this listener is placed.
    pub tag: Option<Atom<'a>>,
    /// Event name.
    pub name: Atom<'a>,
    /// Handler operations.
    pub handler_ops: Vec<'a, UpdateOp<'a>>,
    /// Function name.
    pub handler_fn_name: Option<Atom<'a>>,
}

/// Store @let value.
#[derive(Debug)]
pub struct StoreLetOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target @let declaration.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotId,
    /// Name that the user set when declaring the `@let`.
    pub declared_name: Atom<'a>,
    /// Value expression.
    pub value: Box<'a, IrExpression<'a>>,
}

/// Update conditional.
///
/// This operation orchestrates the conditional logic for @if/@switch blocks.
/// It contains the conditions array and is processed by the conditionals phase.
///
/// Ported from Angular's `ConditionalOp` (update op) in `update.ts`.
#[derive(Debug)]
pub struct ConditionalUpdateOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target conditional (references the first branch's xref).
    pub target: XrefId,
    /// Main test expression (for @switch), or None (for @if).
    pub test: Option<Box<'a, IrExpression<'a>>>,
    /// Conditions for each branch.
    /// Each condition maps a view xref to its corresponding test expression.
    pub conditions: Vec<'a, ConditionalCaseExpr<'a>>,
    /// After processing by conditionals phase, this is a single collapsed expression
    /// that evaluates the conditions and yields the slot number of the template to display.
    pub processed: Option<Box<'a, IrExpression<'a>>>,
    /// Context value for alias capture (e.g., `@if (condition as alias)`).
    pub context_value: Option<Box<'a, IrExpression<'a>>>,
}

/// An i18n expression.
#[derive(Debug)]
pub struct I18nExpressionOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// I18n owner.
    pub i18n_owner: XrefId,
    /// Target element or text.
    pub target: XrefId,
    /// I18n context.
    pub context: XrefId,
    /// Handle (computed).
    pub handle: I18nSlotHandle,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Resolution time.
    pub resolution_time: I18nParamResolutionTime,
    /// Expression usage.
    pub usage: I18nExpressionFor,
    /// Attribute name (for attribute bindings).
    pub name: Atom<'a>,
    /// I18n placeholder name associated with this expression.
    /// This can be None if the expression is part of an ICU placeholder.
    pub i18n_placeholder: Option<Atom<'a>>,
    /// Reference to ICU placeholder op if this expression is part of an ICU.
    pub icu_placeholder: Option<XrefId>,
}

/// Apply i18n expressions.
#[derive(Debug)]
pub struct I18nApplyOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// I18n owner.
    pub i18n_owner: XrefId,
    /// Handle (computed).
    pub handle: I18nSlotHandle,
}

/// ICU placeholder.
///
/// Represents a placeholder within an ICU expression that may contain interpolated values.
/// This is a CreateOp because it replaces Text ops during i18n text extraction.
///
/// Ported from Angular's IcuPlaceholderOp in `ir/src/ops/create.ts`.
#[derive(Debug)]
pub struct IcuPlaceholderOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Placeholder name in the ICU expression.
    pub name: Atom<'a>,
    /// Static string segments to be combined with expression placeholders.
    /// Works like interpolation: strings.len() == expression_placeholders.len() + 1
    pub strings: oxc_allocator::Vec<'a, Atom<'a>>,
    /// Expression placeholders collected from I18nExpression ops.
    /// These are combined with strings to form the full placeholder value.
    pub expression_placeholders: oxc_allocator::Vec<'a, super::i18n_params::I18nParamValue>,
}

/// Intermediate binding.
#[derive(Debug)]
pub struct BindingOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Binding kind.
    pub kind: BindingKind,
    /// Binding name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Unit suffix.
    pub unit: Option<Atom<'a>>,
    /// Security context.
    pub security_context: SecurityContext,
    /// I18n message instance ID.
    ///
    /// Stores the i18n message's instance_id for dedup, not an XrefId.
    pub i18n_message: Option<u32>,
    /// Whether this binding came from a text attribute (e.g., `class="cls"` vs `[class]="expr"`).
    ///
    /// This is used for compatibility with TemplateDefinitionBuilder which treats
    /// `style` and `class` TextAttributes differently from `[attr.style]` and `[attr.class]`.
    pub is_text_attribute: bool,
}

/// Animation binding (CREATE op).
///
/// This operation is converted from AnimationBinding during the convert_animations phase.
/// It emits a syntheticHostProperty instruction that binds an animation trigger to an element.
///
/// In Angular's TypeScript, AnimationOp has handlerOps for callback bodies, but in our
/// simplified implementation we keep the expression directly and emit a return statement.
///
/// Ported from Angular's `AnimationOp` in `ir/src/ops/create.ts`.
#[derive(Debug)]
pub struct AnimationOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Animation name.
    pub name: Atom<'a>,
    /// Animation kind (enter or leave).
    pub animation_kind: AnimationKind,
    /// Handler operations (contains the expression as a return statement).
    pub handler_ops: Vec<'a, UpdateOp<'a>>,
    /// Function name for the handler.
    pub handler_fn_name: Option<Atom<'a>>,
    /// I18n message instance ID.
    ///
    /// Stores the i18n message's instance_id for dedup, not an XrefId.
    pub i18n_message: Option<u32>,
    /// Security context.
    pub security_context: SecurityContext,
    /// Sanitizer function.
    pub sanitizer: Option<Atom<'a>>,
}

/// Animation string binding (CREATE op).
#[derive(Debug)]
pub struct AnimationStringOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Animation name.
    pub name: Atom<'a>,
    /// Animation kind (enter or leave).
    pub animation_kind: AnimationKind,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
}

/// Animation binding.
#[derive(Debug)]
pub struct AnimationBindingOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Animation name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Binding kind.
    pub kind: AnimationBindingKind,
}

/// Animation listener (CREATE op).
#[derive(Debug)]
pub struct AnimationListenerOp<'a> {
    /// Common operation metadata.
    pub base: CreateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Target slot.
    pub target_slot: SlotId,
    /// Tag name of the element on which this listener is placed. Null for host bindings.
    pub tag: Option<Atom<'a>>,
    /// Whether this listener is from a host binding.
    pub host_listener: bool,
    /// Event name.
    pub name: Atom<'a>,
    /// Handler operations.
    pub handler_ops: Vec<'a, UpdateOp<'a>>,
    /// Function name.
    pub handler_fn_name: Option<Atom<'a>>,
    /// Animation phase.
    pub phase: AnimationKind,
    /// Whether this listener uses $event.
    pub consumes_dollar_event: bool,
}

/// Semantic variable declaration in update.
#[derive(Debug)]
pub struct UpdateVariableOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Cross-reference ID.
    pub xref: XrefId,
    /// Variable kind.
    pub kind: SemanticVariableKind,
    /// Variable name.
    pub name: Atom<'a>,
    /// Initializer expression.
    pub initializer: Box<'a, IrExpression<'a>>,
    /// Variable flags.
    pub flags: VariableFlags,
    /// View XrefId for Context and SavedView variables.
    /// For Context: the view whose context is being stored.
    /// For SavedView: the view being saved.
    pub view: Option<XrefId>,
    /// Whether this variable was declared locally within the same view (e.g., @let declarations).
    /// Local variables take precedence during name resolution over non-local variables.
    pub local: bool,
}

/// Control binding.
#[derive(Debug)]
pub struct ControlOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// Target element.
    pub target: XrefId,
    /// Property name.
    pub name: Atom<'a>,
    /// Expression.
    pub expression: Box<'a, IrExpression<'a>>,
    /// Security context.
    pub security_context: SecurityContext,
}

/// An output AST statement for listener handlers.
///
/// This wraps an OutputStatement (ExpressionStatement, ReturnStatement, etc.)
/// and is used to build listener handler function bodies.
///
/// Ported from Angular's `createStatementOp` in `ir/src/ops/shared.ts`.
#[derive(Debug)]
pub struct StatementOp<'a> {
    /// Common operation metadata.
    pub base: UpdateOpBase<'a>,
    /// The output statement.
    pub statement: crate::output::ast::OutputStatement<'a>,
}

// ============================================================================
// Supporting Types
// ============================================================================

/// A local reference attached to an element.
#[derive(Debug)]
pub struct LocalRef<'a> {
    /// Reference name.
    pub name: Atom<'a>,
    /// Target directive/component.
    pub target: Atom<'a>,
}

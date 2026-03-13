//! Component metadata structures.
//!
//! This module defines the metadata extracted from `@Component` decorators.

use oxc_allocator::Vec;
use oxc_span::{Atom, Span};

use super::dependency::R3DependencyMetadata;
use crate::directive::R3InputMetadata;
use crate::output::ast::OutputExpression;

/// Angular version information for feature detection.
///
/// Used to determine version-conditional behavior like the default value
/// for `standalone` (false for v18 and earlier, true for v19+).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AngularVersion {
    /// Major version number (e.g., 19 for Angular 19.0.0).
    pub major: u32,
    /// Minor version number (e.g., 0 for Angular 19.0.0).
    pub minor: u32,
    /// Patch version number (e.g., 0 for Angular 19.0.0).
    pub patch: u32,
}

impl AngularVersion {
    /// Create a new AngularVersion.
    pub fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// Check if this version supports implicit standalone default (v19.0.0+).
    ///
    /// Angular v19 and later default `standalone` to `true` when not explicitly set.
    /// Earlier versions default to `false`.
    pub fn supports_implicit_standalone(&self) -> bool {
        self.major >= 19
    }

    /// Check if this version supports `ɵɵconditionalCreate`/`ɵɵconditionalBranchCreate` (v20.0.0+).
    ///
    /// Angular v20 introduced `ɵɵconditionalCreate` and `ɵɵconditionalBranchCreate`
    /// instructions for `@if`/`@switch` blocks. Earlier versions use `ɵɵtemplate` instead.
    pub fn supports_conditional_create(&self) -> bool {
        self.major >= 20
    }

    /// Parse a version string like "19.0.0" or "19.0.0-rc.1".
    ///
    /// Returns `None` if the version string is invalid.
    pub fn parse(version_str: &str) -> Option<Self> {
        // Handle placeholder version (means latest/head)
        if version_str == "0.0.0-PLACEHOLDER" {
            return Some(Self::new(u32::MAX, 0, 0)); // Treat as latest
        }

        // Strip any prerelease suffix: "19.0.0-rc.1" -> "19.0.0"
        let base_version = version_str.split('-').next()?;

        let mut parts = base_version.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        Some(Self::new(major, minor, patch))
    }
}

/// View encapsulation strategy for component styles.
///
/// Matches Angular's `ViewEncapsulation` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ViewEncapsulation {
    /// Emulated encapsulation using attribute selectors.
    /// This is the default behavior.
    #[default]
    Emulated,
    /// No encapsulation - styles are global.
    None,
    /// Native Shadow DOM encapsulation.
    ShadowDom,
}

/// Change detection strategy for the component.
///
/// Matches Angular's `ChangeDetectionStrategy` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ChangeDetectionStrategy {
    /// Check the component on every change detection cycle.
    #[default]
    Default,
    /// Only check when inputs change or events are triggered.
    OnPush,
}

/// Metadata extracted from an `@Component` decorator.
///
/// This represents all the configuration properties that can be
/// specified in the `@Component({...})` decorator.
#[derive(Debug)]
pub struct ComponentMetadata<'a> {
    /// The name of the component class.
    pub class_name: Atom<'a>,

    /// The span of the class declaration.
    pub class_span: Span,

    /// The CSS selector for this component.
    pub selector: Option<Atom<'a>>,

    /// Inline template string.
    pub template: Option<Atom<'a>>,

    /// URL to an external template file.
    pub template_url: Option<Atom<'a>>,

    /// Inline styles array.
    pub styles: Vec<'a, Atom<'a>>,

    /// URLs to external stylesheet files.
    pub style_urls: Vec<'a, Atom<'a>>,

    /// Whether this is a standalone component.
    pub standalone: bool,

    /// View encapsulation mode.
    pub encapsulation: ViewEncapsulation,

    /// Change detection strategy.
    pub change_detection: ChangeDetectionStrategy,

    /// Host bindings and listeners.
    pub host: Option<HostMetadata<'a>>,

    /// Component imports (for standalone components).
    pub imports: Vec<'a, Atom<'a>>,

    /// Exported names for template references.
    ///
    /// In Angular, `exportAs` can be a comma-separated string (e.g., "foo, bar"),
    /// which is split into an array and emitted as `exportAs: ["foo", "bar"]`.
    pub export_as: Vec<'a, Atom<'a>>,

    /// Whether to preserve whitespace in templates.
    pub preserve_whitespaces: bool,

    /// Constructor parameter dependencies for DI.
    ///
    /// - `Some(deps)`: Constructor exists, deps contains parameters to inject
    /// - `None`: No constructor found, use inherited factory pattern
    ///
    /// This distinction is important because:
    /// - `Some([])` (empty vec) = Constructor with 0 params → `new Class()`
    /// - `None` = No constructor → Use `ɵɵgetInheritedFactory` IIFE pattern
    pub constructor_deps: Option<Vec<'a, R3DependencyMetadata<'a>>>,

    /// Inputs of the component (@Input decorators).
    ///
    /// Extracted from class property decorators like:
    /// - `@Input() value: string;`
    /// - `@Input('alias') value: string;`
    /// - `@Input({ required: true }) value: string;`
    pub inputs: Vec<'a, R3InputMetadata<'a>>,

    /// Outputs of the component (@Output decorators).
    ///
    /// Each entry is a tuple of (class_property_name, binding_property_name).
    /// Extracted from class property decorators like:
    /// - `@Output() valueChange = new EventEmitter<string>();`
    /// - `@Output('changed') valueChange = new EventEmitter<string>();`
    pub outputs: Vec<'a, (Atom<'a>, Atom<'a>)>,

    // =========================================================================
    // Feature-related fields
    // See: packages/compiler/src/render3/view/compiler.ts:119-161
    // =========================================================================
    /// Providers for dependency injection.
    ///
    /// Corresponds to the `providers` property in `@Component`.
    /// Used to generate `ɵɵProvidersFeature`.
    ///
    /// Stores the full expression (e.g., `[{provide: TOKEN, useFactory: Factory}]`)
    /// not just identifier names.
    pub providers: Option<OutputExpression<'a>>,

    /// View providers for dependency injection.
    ///
    /// Corresponds to the `viewProviders` property in `@Component`.
    /// Used with `providers` to generate `ɵɵProvidersFeature`.
    ///
    /// Stores the full expression similar to `providers`.
    pub view_providers: Option<OutputExpression<'a>>,

    /// Host directives configuration.
    ///
    /// Corresponds to the `hostDirectives` property in `@Component`.
    /// Used to generate `ɵɵHostDirectivesFeature`.
    pub host_directives: Vec<'a, HostDirectiveMetadata<'a>>,

    /// Whether the component class extends another directive/component.
    ///
    /// When true, generates `ɵɵInheritDefinitionFeature` to inherit
    /// inputs, outputs, and host bindings from the parent class.
    pub uses_inheritance: bool,

    /// Lifecycle hooks implemented by the component.
    pub lifecycle: LifecycleMetadata,

    /// External stylesheet URLs that were resolved and need to be loaded.
    ///
    /// Corresponds to resolved `styleUrls`. Used to generate `ɵɵExternalStylesFeature`.
    pub external_styles: Vec<'a, Atom<'a>>,

    // =========================================================================
    // Template Dependency Fields
    // See: packages/compiler/src/render3/view/compiler.ts:272-289
    // =========================================================================
    /// Template dependencies (directives and pipes used in the template).
    ///
    /// These are extracted from the template during compilation and
    /// used to generate the `dependencies` field in the component definition.
    pub declarations: Vec<'a, TemplateDependency<'a>>,

    /// How to emit the declarations list.
    ///
    /// Determines whether dependencies are emitted directly, wrapped in
    /// a closure (for forward references), or resolved at runtime.
    pub declaration_list_emit_mode: DeclarationListEmitMode,

    /// Raw imports expression for standalone components (local compilation).
    ///
    /// Used with `RuntimeResolved` emit mode to pass the imports array
    /// to `ɵɵgetComponentDepsFactory` at runtime.
    ///
    /// This stores the full expression from the `imports` property, which can be:
    /// - An array literal: `[AsyncPipe, DatePipe]`
    /// - A variable reference: `MY_IMPORTS`
    /// - A spread expression: `[...SHARED_IMPORTS, MyPipe]`
    pub raw_imports: Option<OutputExpression<'a>>,

    /// Animation triggers for the component.
    ///
    /// Corresponds to the `animations` property in `@Component`.
    /// Generated as `data: {animation: [...]}` in the component definition.
    ///
    /// Stores the full expression (e.g., `[trigger('open', [transition(...)])]`)
    /// not just identifier names.
    pub animations: Option<OutputExpression<'a>>,

    /// Schemas for the component.
    ///
    /// Corresponds to the `schemas` property in `@Component`.
    /// Common values are `CUSTOM_ELEMENTS_SCHEMA` and `NO_ERRORS_SCHEMA`.
    /// These are stored as identifier names.
    pub schemas: Vec<'a, Atom<'a>>,

    /// Whether this component uses signal-based inputs.
    ///
    /// This is set to `true` only when the decorator explicitly contains `signals: true`.
    /// When true, the component definition should include `signals: true`.
    ///
    /// See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:382-390
    pub is_signal: bool,
}

/// Metadata for a host directive.
///
/// Corresponds to TypeScript's `HostDirectiveDef` in the compiler.
/// See: packages/compiler/src/render3/view/compiler.ts:683-723
#[derive(Debug)]
pub struct HostDirectiveMetadata<'a> {
    /// The directive class name.
    pub directive: Atom<'a>,

    /// The source module of the directive (e.g., "@angular/common", "./my-directive").
    /// Used to generate proper namespace aliases for imported host directives.
    /// `None` for local directives defined in the same file.
    pub source_module: Option<Atom<'a>>,

    /// Input mappings: (publicName, internalName) pairs.
    /// Empty if no inputs are exposed.
    pub inputs: Vec<'a, (Atom<'a>, Atom<'a>)>,

    /// Output mappings: (publicName, internalName) pairs.
    /// Empty if no outputs are exposed.
    pub outputs: Vec<'a, (Atom<'a>, Atom<'a>)>,

    /// Whether this is a forward reference (requires wrapping in a function).
    pub is_forward_reference: bool,
}

impl<'a> HostDirectiveMetadata<'a> {
    /// Create a new HostDirectiveMetadata.
    pub fn new(allocator: &'a oxc_allocator::Allocator, directive: Atom<'a>) -> Self {
        Self {
            directive,
            source_module: None,
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
            is_forward_reference: false,
        }
    }

    /// Check if this directive has input or output mappings.
    pub fn has_mappings(&self) -> bool {
        !self.inputs.is_empty() || !self.outputs.is_empty()
    }

    /// Set the source module for this host directive.
    pub fn with_source_module(mut self, source_module: Atom<'a>) -> Self {
        self.source_module = Some(source_module);
        self
    }
}

/// Lifecycle hooks metadata.
///
/// Tracks which lifecycle hooks are implemented by the component.
/// See: packages/compiler/src/render3/view/compiler.ts:143
#[derive(Debug, Clone, Copy, Default)]
pub struct LifecycleMetadata {
    /// Whether the component implements `ngOnChanges`.
    ///
    /// When true, generates `ɵɵNgOnChangesFeature` to enable
    /// the SimpleChanges tracking.
    pub uses_on_changes: bool,
}

// =============================================================================
// Template Dependencies
// See: packages/compiler/src/render3/view/api.ts:149-396
// =============================================================================

/// How the declarations list (directives/pipes) is emitted.
///
/// See: packages/compiler/src/render3/view/api.ts:149-190
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeclarationListEmitMode {
    /// The list of declarations is emitted directly.
    /// `dependencies: [MyDir, MyPipe]`
    #[default]
    Direct,

    /// Wrapped in a closure for forward references.
    /// `dependencies: function() { return [MyDir, ForwardDir]; }`
    Closure,

    /// Closure with resolveForwardRef mapping for JIT.
    /// `dependencies: function() { return [MyDir].map(ng.resolveForwardRef); }`
    ClosureResolved,

    /// Dependencies resolved at runtime using getComponentDepsFactory.
    /// Used in local compilation mode.
    /// `dependencies: ɵɵgetComponentDepsFactory(Component, rawImports)`
    RuntimeResolved,
}

/// Kind of template dependency.
///
/// See: packages/compiler/src/render3/view/api.ts:329-333
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplateDependencyKind {
    /// A directive used in the template.
    Directive,
    /// A pipe used in the template.
    Pipe,
    /// An NgModule dependency.
    NgModule,
}

/// Metadata for a template dependency (directive or pipe).
///
/// See: packages/compiler/src/render3/view/api.ts:335-396
#[derive(Debug)]
pub struct TemplateDependency<'a> {
    /// The kind of dependency (directive, pipe, or NgModule).
    pub kind: TemplateDependencyKind,

    /// The type expression (class name or import reference).
    pub type_name: Atom<'a>,

    /// The source module of the dependency (e.g., "@angular/common", "./my-directive").
    /// Used to generate proper namespace aliases for imported dependencies.
    /// `None` for local dependencies defined in the same file.
    pub source_module: Option<Atom<'a>>,

    /// For directives: the CSS selector.
    pub selector: Option<Atom<'a>>,

    /// For directives: input property names.
    pub inputs: Vec<'a, Atom<'a>>,

    /// For directives: output property names.
    pub outputs: Vec<'a, Atom<'a>>,

    /// For directives: export names (exportAs).
    pub export_as: Vec<'a, Atom<'a>>,

    /// For directives: whether this is a component.
    pub is_component: bool,

    /// For pipes: the pipe name used in templates.
    pub pipe_name: Option<Atom<'a>>,

    /// Whether this is a forward reference.
    pub is_forward_reference: bool,
}

impl<'a> TemplateDependency<'a> {
    /// Create a new directive dependency.
    pub fn directive(
        allocator: &'a oxc_allocator::Allocator,
        type_name: Atom<'a>,
        selector: Atom<'a>,
        is_component: bool,
    ) -> Self {
        Self {
            kind: TemplateDependencyKind::Directive,
            type_name,
            source_module: None,
            selector: Some(selector),
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
            export_as: Vec::new_in(allocator),
            is_component,
            pipe_name: None,
            is_forward_reference: false,
        }
    }

    /// Create a new pipe dependency.
    pub fn pipe(
        allocator: &'a oxc_allocator::Allocator,
        type_name: Atom<'a>,
        pipe_name: Atom<'a>,
    ) -> Self {
        Self {
            kind: TemplateDependencyKind::Pipe,
            type_name,
            source_module: None,
            selector: None,
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
            export_as: Vec::new_in(allocator),
            is_component: false,
            pipe_name: Some(pipe_name),
            is_forward_reference: false,
        }
    }

    /// Mark this dependency as a forward reference.
    pub fn with_forward_reference(mut self) -> Self {
        self.is_forward_reference = true;
        self
    }

    /// Set the source module for this dependency.
    pub fn with_source_module(mut self, source_module: Atom<'a>) -> Self {
        self.source_module = Some(source_module);
        self
    }
}

/// Host metadata for bindings and listeners.
///
/// Reference: packages/compiler/src/render3/view/api.ts - R3HostMetadata
#[derive(Debug)]
pub struct HostMetadata<'a> {
    /// Host property bindings: `{ '[class.active]': 'isActive' }`
    pub properties: Vec<'a, (Atom<'a>, Atom<'a>)>,

    /// Host attribute bindings: `{ 'role': 'button' }`
    pub attributes: Vec<'a, (Atom<'a>, Atom<'a>)>,

    /// Host event listeners: `{ '(click)': 'onClick()' }`
    pub listeners: Vec<'a, (Atom<'a>, Atom<'a>)>,

    /// Special attribute for static class binding: `{ 'class': 'foo bar' }`
    /// Captured separately to generate class instructions during compilation.
    pub class_attr: Option<Atom<'a>>,

    /// Special attribute for static style binding: `{ 'style': 'color: red' }`
    /// Captured separately to generate style instructions during compilation.
    pub style_attr: Option<Atom<'a>>,
}

impl<'a> HostMetadata<'a> {
    /// Create a new empty HostMetadata.
    pub fn new(allocator: &'a oxc_allocator::Allocator) -> Self {
        Self {
            properties: Vec::new_in(allocator),
            attributes: Vec::new_in(allocator),
            listeners: Vec::new_in(allocator),
            class_attr: None,
            style_attr: None,
        }
    }
}

impl<'a> ComponentMetadata<'a> {
    /// Create a new ComponentMetadata with default values.
    ///
    /// The `implicit_standalone` parameter determines the default value for `standalone`
    /// when not explicitly set in the decorator. This should be:
    /// - `true` for Angular v19+
    /// - `false` for Angular v18 and earlier
    /// - `true` when the Angular version is unknown (assume latest)
    pub fn new(
        allocator: &'a oxc_allocator::Allocator,
        class_name: Atom<'a>,
        class_span: Span,
        implicit_standalone: bool,
    ) -> Self {
        Self {
            class_name,
            class_span,
            selector: None,
            template: None,
            template_url: None,
            styles: Vec::new_in(allocator),
            style_urls: Vec::new_in(allocator),
            standalone: implicit_standalone,
            encapsulation: ViewEncapsulation::default(),
            change_detection: ChangeDetectionStrategy::default(),
            host: None,
            imports: Vec::new_in(allocator),
            export_as: Vec::new_in(allocator),
            preserve_whitespaces: false,
            constructor_deps: None,
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
            // Feature-related fields
            providers: None,
            view_providers: None,
            host_directives: Vec::new_in(allocator),
            uses_inheritance: false,
            lifecycle: LifecycleMetadata::default(),
            external_styles: Vec::new_in(allocator),
            // Template dependency fields
            declarations: Vec::new_in(allocator),
            declaration_list_emit_mode: DeclarationListEmitMode::default(),
            raw_imports: None,
            animations: None,
            schemas: Vec::new_in(allocator),
            is_signal: false,
        }
    }

    /// Returns true if this component has an inline template.
    pub fn has_inline_template(&self) -> bool {
        self.template.is_some()
    }

    /// Returns true if this component has an external template.
    pub fn has_external_template(&self) -> bool {
        self.template_url.is_some()
    }

    /// Returns true if this component has any styles (inline or external).
    pub fn has_styles(&self) -> bool {
        !self.styles.is_empty() || !self.style_urls.is_empty()
    }

    /// Generate a unique component ID for HMR.
    ///
    /// Format: `{file_path}@{class_name}`
    pub fn component_id(&self, file_path: &str) -> String {
        format!("{}@{}", file_path, self.class_name)
    }
}

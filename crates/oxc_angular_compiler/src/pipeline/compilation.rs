//! Compilation job and view compilation unit.
//!
//! The CompilationJob holds all state for compiling a single component template.
//! It contains multiple ViewCompilationUnits, one for each view in the template
//! (including embedded views for control flow blocks).
//!
//! Ported from Angular's `template/pipeline/src/compilation.ts`.

use indexmap::IndexMap;
use oxc_allocator::{Allocator, Box, Vec};
use oxc_diagnostics::OxcDiagnostic;
use oxc_span::{Atom, Span};
use rustc_hash::{FxBuildHasher, FxHashMap};

use crate::AngularVersion;
use crate::ir::enums::CompatibilityMode;
use crate::ir::list::{CreateOpList, UpdateOpList};
use crate::ir::ops::XrefId;
use crate::output::ast::OutputStatement;

use super::constant_pool::ConstantPool;
use super::expression_store::ExpressionStore;

/// Possible modes in which a component's template can be compiled.
///
/// Ported from Angular's `TemplateCompilationMode` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TemplateCompilationMode {
    /// Supports the full instruction set, including directives.
    /// This is the default mode.
    #[default]
    Full,

    /// Uses a narrower instruction set that doesn't support directives.
    ///
    /// This mode allows optimizations because the compiler knows that:
    /// - Property bindings go directly to DOM properties (not directive inputs)
    /// - Listeners go directly to DOM events (not directive outputs)
    /// - No directive matching is needed at runtime
    ///
    /// Used when the component is standalone and has no directive dependencies.
    DomOnly,
}

/// Defines how dynamic imports for deferred dependencies should be emitted.
///
/// Ported from Angular's `DeferBlockDepsEmitMode` enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DeferBlockDepsEmitMode {
    /// Dynamic imports are grouped on per-block basis.
    ///
    /// This is used in full compilation mode, when compiler has more information
    /// about particular dependencies that belong to this block.
    #[default]
    PerBlock,

    /// Dynamic imports are grouped on per-component basis.
    ///
    /// In local compilation, compiler doesn't have enough information to determine
    /// which deferred dependencies belong to which block. In this case we group all
    /// dynamic imports into a single file on per-component basis.
    PerComponent,
}

/// Metadata about defer blocks in a component.
///
/// Ported from Angular's `R3ComponentDeferMetadata` type.
/// In Angular TS, the blocks map uses the AST node as the key.
/// In Rust, we use the source Span since it uniquely identifies each defer block.
#[derive(Debug)]
pub enum DeferMetadata<'a> {
    /// Per-block mode with resolver expressions for each defer block.
    PerBlock {
        /// Map from defer block source span to the resolver expression (if any).
        /// The key is the source_span of the R3DeferredBlock, which uniquely identifies it.
        blocks: FxHashMap<Span, Option<crate::output::ast::OutputExpression<'a>>>,
    },
    /// Per-component mode with a single dependencies function.
    PerComponent {
        /// The dependencies function expression for all defer blocks.
        dependencies_fn: Option<crate::output::ast::OutputExpression<'a>>,
    },
}

/// Metadata associated with an i18n message.
#[derive(Debug)]
pub struct I18nMessageMetadata<'a> {
    /// Message ID (computed).
    pub message_id: Option<Atom<'a>>,
    /// Custom ID supplied by the author.
    pub custom_id: Option<Atom<'a>>,
    /// Message meaning for disambiguation.
    pub meaning: Option<Atom<'a>>,
    /// Message description for translators.
    pub description: Option<Atom<'a>>,
    /// Legacy message IDs.
    pub legacy_ids: Vec<'a, Atom<'a>>,
    /// The serialized message string for goog.getMsg and $localize.
    /// Contains the message text with placeholder markers like "{$interpolation}".
    pub message_string: Option<Atom<'a>>,
}

/// A complete compilation job for a single component template.
///
/// The compilation job holds all views (root + embedded) and coordinates
/// the 67 transformation phases that process the IR.
pub struct ComponentCompilationJob<'a> {
    /// The allocator for this compilation.
    pub allocator: &'a Allocator,
    /// Name of the component being compiled.
    pub component_name: Atom<'a>,
    /// Constant pool for deduplication.
    pub pool: ConstantPool<'a>,
    /// Expression store for managing expressions by reference.
    /// Uses the Reference + Index pattern to avoid cloning expressions.
    pub expressions: ExpressionStore<'a>,
    /// The root view of the template.
    pub root: ViewCompilationUnit<'a>,
    /// All views indexed by their cross-reference ID.
    ///
    /// Uses IndexMap to preserve insertion order, ensuring deterministic
    /// iteration for projection slots, naming, and emission.
    pub views: IndexMap<XrefId, Box<'a, ViewCompilationUnit<'a>>, FxBuildHasher>,
    /// Constants extracted from the template.
    pub consts: Vec<'a, ConstValue<'a>>,
    /// Initialization statements needed to set up the consts.
    ///
    /// These statements are executed before the consts array is constructed.
    /// Used for i18n dual-mode (Closure + $localize) declarations.
    pub consts_initializers: Vec<'a, OutputStatement<'a>>,
    /// Next cross-reference ID to allocate.
    next_xref_id: u32,
    /// Compatibility mode for output.
    pub compatibility_mode: CompatibilityMode,
    /// Template compilation mode (Full or DomOnly).
    ///
    /// In DomOnly mode, the compiler uses optimized DOM-only instructions
    /// (ɵɵdomElement, ɵɵdomProperty, etc.) that skip directive matching.
    /// This is used when the component is standalone and has no directive dependencies.
    pub mode: TemplateCompilationMode,
    /// Whether this is for an i18n template.
    pub is_i18n_template: bool,
    /// Metadata for i18n messages keyed by instance_id.
    ///
    /// The instance_id is a unique u32 assigned to each i18n message during parsing.
    /// This avoids allocating xrefs during ingest for i18n messages on attribute bindings,
    /// matching Angular TS which stores direct object references on BindingOp.i18nMessage.
    pub i18n_message_metadata: FxHashMap<u32, I18nMessageMetadata<'a>>,
    /// Whether to use external message IDs in Closure Compiler variable names.
    ///
    /// When true, generates variable names like `MSG_EXTERNAL_abc123$$SUFFIX`.
    /// When false, uses file-based naming like `MSG_SUFFIX_0`.
    /// This is used for Closure Compiler's `goog.getMsg` translation system.
    pub i18n_use_external_ids: bool,
    /// Relative path to the context file for i18n suffix generation.
    ///
    /// Used to generate unique, file-based variable names for i18n translations.
    /// The path is sanitized to create a valid identifier suffix.
    pub relative_context_file_path: Option<Atom<'a>>,
    /// Relocation entries.
    pub relocation_entries: Vec<'a, RelocationEntry>,
    /// Whether to attach debug source locations.
    /// When enabled, the `attach_source_locations` phase will generate
    /// `ɵɵsourceLocation` calls for debugging.
    pub enable_debug_locations: bool,
    /// Relative path to the template file for source location debugging.
    /// Required when `enable_debug_locations` is true.
    pub relative_template_path: Option<Atom<'a>>,
    /// Template source text for computing line/column from byte offsets.
    /// Required when `enable_debug_locations` is true.
    pub template_source: Option<&'a str>,
    /// Defer block metadata.
    ///
    /// Controls how defer block dependencies are emitted (per-block or per-component).
    /// Ported from Angular's `R3ComponentDeferMetadata`.
    pub defer_meta: DeferMetadata<'a>,
    /// Reference to the deferrable dependencies function when using PerComponent mode.
    ///
    /// This is the `allDeferrableDepsFn` from Angular's ingest.ts.
    /// Used when `defer_meta` is `PerComponent` to reference the shared dependencies function.
    pub all_deferrable_deps_fn: Option<crate::output::ast::OutputExpression<'a>>,
    /// Content selectors for ng-content slots.
    ///
    /// Causes `ngContentSelectors` to be emitted in the component definition.
    /// This is populated by the `generate_projection_def` phase.
    pub content_selectors: Option<crate::output::ast::OutputExpression<'a>>,
    /// Angular version for feature-gated instruction selection.
    ///
    /// When set to a version < 20, the compiler emits `ɵɵtemplate` instead of
    /// `ɵɵconditionalCreate`/`ɵɵconditionalBranchCreate` for `@if`/`@switch` blocks.
    /// When `None`, assumes latest Angular version (v20+ behavior).
    pub angular_version: Option<AngularVersion>,
    /// Diagnostics collected during compilation.
    pub diagnostics: std::vec::Vec<OxcDiagnostic>,
}

impl<'a> ComponentCompilationJob<'a> {
    /// Creates a new compilation job.
    pub fn new(allocator: &'a Allocator, component_name: Atom<'a>) -> Self {
        Self::with_pool_starting_index(allocator, component_name, 0)
    }

    /// Creates a new compilation job with a specific constant pool starting index.
    ///
    /// This is used when compiling multiple components in the same file to ensure
    /// constant names don't conflict. Each component continues from where the
    /// previous component's pool left off.
    ///
    /// For example, if component 1 uses _c0, _c1, _c2, then component 2 should
    /// be created with `pool_starting_index: 3` to start with _c3.
    pub fn with_pool_starting_index(
        allocator: &'a Allocator,
        component_name: Atom<'a>,
        pool_starting_index: u32,
    ) -> Self {
        let root_xref = XrefId::new(0);
        let root = ViewCompilationUnit::new(allocator, root_xref, None);

        Self {
            allocator,
            component_name,
            pool: ConstantPool::with_starting_index(allocator, pool_starting_index),
            expressions: ExpressionStore::new(allocator),
            root,
            views: IndexMap::with_hasher(FxBuildHasher),
            consts: Vec::new_in(allocator),
            consts_initializers: Vec::new_in(allocator),
            next_xref_id: 1, // 0 is reserved for root
            compatibility_mode: CompatibilityMode::TemplateDefinitionBuilder,
            mode: TemplateCompilationMode::default(),
            is_i18n_template: false,
            i18n_message_metadata: FxHashMap::default(),
            i18n_use_external_ids: true, // Default matches Angular's JIT behavior
            relative_context_file_path: None,
            relocation_entries: Vec::new_in(allocator),
            enable_debug_locations: false,
            relative_template_path: None,
            template_source: None,
            defer_meta: DeferMetadata::PerBlock { blocks: FxHashMap::default() },
            all_deferrable_deps_fn: None,
            content_selectors: None,
            angular_version: None,
            diagnostics: std::vec::Vec::new(),
        }
    }

    /// Sets the template compilation mode.
    ///
    /// Use `TemplateCompilationMode::DomOnly` when the component is standalone
    /// and has no directive dependencies for optimized DOM-only output.
    pub fn with_mode(mut self, mode: TemplateCompilationMode) -> Self {
        self.mode = mode;
        self
    }

    /// Check if `ɵɵconditionalCreate` is supported (Angular 20+).
    ///
    /// Returns `true` for Angular 20+ or when version is unknown (None = latest).
    /// Returns `false` for Angular 19 and earlier, which use `ɵɵtemplate` instead.
    pub fn supports_conditional_create(&self) -> bool {
        self.angular_version.map_or(true, |v: AngularVersion| v.supports_conditional_create())
    }

    /// Allocates a new cross-reference ID.
    pub fn allocate_xref_id(&mut self) -> XrefId {
        let id = XrefId::new(self.next_xref_id);
        self.next_xref_id += 1;
        id
    }

    /// Stores an expression and returns its ID.
    ///
    /// Use this instead of inline expressions to avoid cloning.
    pub fn store_expression(
        &mut self,
        expr: crate::ast::expression::AngularExpression<'a>,
    ) -> super::expression_store::ExpressionId {
        self.expressions.store(expr)
    }

    /// Retrieves an expression by its ID.
    pub fn get_expression(
        &self,
        id: super::expression_store::ExpressionId,
    ) -> &crate::ast::expression::AngularExpression<'a> {
        self.expressions.get(id)
    }

    /// Creates a new view and returns its cross-reference ID.
    pub fn allocate_view(&mut self, parent: Option<XrefId>) -> XrefId {
        let xref = self.allocate_xref_id();
        let view = ViewCompilationUnit::new(self.allocator, xref, parent);
        let boxed = Box::new_in(view, self.allocator);
        self.views.insert(xref, boxed);
        xref
    }

    /// Returns a reference to a view by its cross-reference ID.
    pub fn view(&self, xref: XrefId) -> Option<&ViewCompilationUnit<'a>> {
        if xref.0 == 0 { Some(&self.root) } else { self.views.get(&xref).map(|b| b.as_ref()) }
    }

    /// Returns a mutable reference to a view by its cross-reference ID.
    pub fn view_mut(&mut self, xref: XrefId) -> Option<&mut ViewCompilationUnit<'a>> {
        if xref.0 == 0 {
            Some(&mut self.root)
        } else {
            self.views.get_mut(&xref).map(|b| b.as_mut())
        }
    }

    /// Iterates over all views in the compilation job.
    pub fn all_views(&self) -> impl Iterator<Item = &ViewCompilationUnit<'a>> {
        std::iter::once(&self.root).chain(self.views.values().map(|b| b.as_ref()))
    }

    /// Iterates mutably over all views in the compilation job.
    pub fn all_views_mut(&mut self) -> AllViewsMut<'_, 'a> {
        AllViewsMut { root: Some(&mut self.root), views_iter: self.views.values_mut() }
    }

    /// Adds a constant to the consts array, deduplicating if an equivalent value exists.
    ///
    /// Returns the index of the constant (either existing or newly added).
    pub fn add_const(&mut self, value: ConstValue<'a>) -> u32 {
        // Check for existing equivalent constant
        for (idx, existing) in self.consts.iter().enumerate() {
            if existing.is_equivalent(&value) {
                return idx as u32;
            }
        }

        // No equivalent found, add new constant
        let index = self.consts.len() as u32;
        self.consts.push(value);
        index
    }

    /// Adds a constant to the consts array with optional initializer statements.
    ///
    /// The initializer statements are executed before the consts array is constructed.
    /// This is used for i18n dual-mode (Closure + $localize) declarations where
    /// we need to declare variables and set up conditional code before the const.
    pub fn add_const_with_initializers(
        &mut self,
        value: ConstValue<'a>,
        initializers: impl IntoIterator<Item = OutputStatement<'a>>,
    ) -> u32 {
        self.consts_initializers.extend(initializers);
        self.add_const(value)
    }
}

/// Mutable iterator over all views.
pub struct AllViewsMut<'b, 'a> {
    root: Option<&'b mut ViewCompilationUnit<'a>>,
    views_iter: indexmap::map::ValuesMut<'b, XrefId, Box<'a, ViewCompilationUnit<'a>>>,
}

impl<'b, 'a> Iterator for AllViewsMut<'b, 'a> {
    type Item = &'b mut ViewCompilationUnit<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(root) = self.root.take() {
            Some(root)
        } else {
            self.views_iter.next().map(|b| b.as_mut())
        }
    }
}

/// A single view in the compilation.
///
/// Each view corresponds to either:
/// - The root template
/// - An embedded template (ng-template, structural directive)
/// - A control flow block (@if, @for, @switch, @defer branches)
pub struct ViewCompilationUnit<'a> {
    /// Cross-reference ID for this view.
    pub xref: XrefId,
    /// Parent view, if any.
    pub parent: Option<XrefId>,
    /// Create-time operations.
    pub create: CreateOpList<'a>,
    /// Update-time operations.
    pub update: UpdateOpList<'a>,
    /// Create-time statements (populated by reify phase).
    pub create_statements: Vec<'a, OutputStatement<'a>>,
    /// Update-time statements (populated by reify phase).
    pub update_statements: Vec<'a, OutputStatement<'a>>,
    /// Number of variable slots needed.
    pub vars: Option<u32>,
    /// Generated function name.
    pub fn_name: Option<Atom<'a>>,
    /// Declaration count for template compatibility.
    pub decl_count: Option<u32>,
    /// First child element/template xref.
    pub first_child: Option<XrefId>,
    /// Context variables available in this view (e.g., $implicit, $index).
    pub context_variables: Vec<'a, ContextVariable<'a>>,
    /// Alias variables available in this view.
    ///
    /// Aliases are computed expressions that are inlined at every usage site.
    /// Used for @for loop computed variables like $first, $last, $even, $odd.
    pub aliases: Vec<'a, AliasVariable<'a>>,
    /// Arrow functions found in this view.
    ///
    /// This is a shortcut so we don't need to traverse all the ops to find functions.
    /// Populated by the generateArrowFunctions phase.
    ///
    /// Ported from Angular's `unit.functions` in `compilation.ts`.
    /// Uses raw pointers to ArrowFunctionExpr since they're stored in the allocator.
    pub functions: Vec<'a, *mut crate::ir::expression::ArrowFunctionExpr<'a>>,
}

impl<'a> ViewCompilationUnit<'a> {
    /// Creates a new view compilation unit.
    pub fn new(allocator: &'a Allocator, xref: XrefId, parent: Option<XrefId>) -> Self {
        Self {
            xref,
            parent,
            create: CreateOpList::new(allocator),
            update: UpdateOpList::new(allocator),
            create_statements: Vec::new_in(allocator),
            update_statements: Vec::new_in(allocator),
            vars: None,
            fn_name: None,
            decl_count: None,
            first_child: None,
            context_variables: Vec::new_in(allocator),
            aliases: Vec::new_in(allocator),
            functions: Vec::new_in(allocator),
        }
    }
}

/// When referenced in the template's context parameters, this indicates a reference to the entire
/// context object, rather than a specific parameter.
///
/// Used for conditional expression aliases (e.g., `@if (alias = expr)`) where the alias
/// should resolve to `ctx` directly, not `ctx.$implicit` or any other property.
///
/// Ported from Angular's `ir.CTX_REF` in `variable.ts`.
pub const CTX_REF: &str = "CTX_REF_MARKER";

/// A context variable in a view.
#[derive(Debug)]
pub struct ContextVariable<'a> {
    /// Variable name (the user-defined identifier).
    pub name: Atom<'a>,
    /// Variable value (the context property name, e.g., "$implicit").
    /// If this equals `CTX_REF`, the variable represents the entire context object.
    pub value: Atom<'a>,
    /// Cross-reference to the variable's origin.
    pub xref: XrefId,
}

/// An alias variable in a view.
///
/// Alias variables are inlined at every usage site. They are used for
/// computed context variables in @for loops (e.g., $first = $index === 0).
///
/// Ported from Angular's `ir.AliasVariable`.
#[derive(Debug)]
pub struct AliasVariable<'a> {
    /// The user-visible identifier for this alias.
    pub identifier: Atom<'a>,
    /// The expression that computes this alias's value.
    /// This expression is cloned and inlined at every usage site.
    pub expression: crate::ir::expression::IrExpression<'a>,
}

/// A constant value in the consts array.
#[derive(Debug)]
pub enum ConstValue<'a> {
    /// A string constant.
    String(Atom<'a>),
    /// An array of constants (attribute arrays).
    Array(Vec<'a, ConstValue<'a>>),
    /// A number constant.
    Number(f64),
    /// A boolean constant.
    Boolean(bool),
    /// Null constant.
    Null,
    /// An external reference (e.g., to a directive).
    External(ExternalRef<'a>),
    /// An output expression (for complex const expressions).
    Expression(crate::output::ast::OutputExpression<'a>),
}

impl<'a> ConstValue<'a> {
    /// Checks if two const values are equivalent for deduplication purposes.
    ///
    /// This is used to avoid duplicating identical constants in the consts array.
    pub fn is_equivalent(&self, other: &ConstValue<'a>) -> bool {
        match (self, other) {
            (ConstValue::String(a), ConstValue::String(b)) => a == b,
            (ConstValue::Number(a), ConstValue::Number(b)) => {
                // Handle NaN specially (NaN != NaN, but for dedup purposes they're equivalent)
                (a.is_nan() && b.is_nan()) || a == b
            }
            (ConstValue::Boolean(a), ConstValue::Boolean(b)) => a == b,
            (ConstValue::Null, ConstValue::Null) => true,
            (ConstValue::Array(a), ConstValue::Array(b)) => {
                a.len() == b.len() && a.iter().zip(b.iter()).all(|(x, y)| x.is_equivalent(y))
            }
            (ConstValue::External(a), ConstValue::External(b)) => {
                a.module_name == b.module_name && a.name == b.name
            }
            (ConstValue::Expression(a), ConstValue::Expression(b)) => a.is_equivalent(b),
            _ => false,
        }
    }
}

/// An external reference for runtime lookup.
#[derive(Debug)]
pub struct ExternalRef<'a> {
    /// Module name (e.g., "@angular/core").
    pub module_name: Atom<'a>,
    /// Export name.
    pub name: Atom<'a>,
}

/// A relocation entry for defer blocks.
#[derive(Debug)]
pub struct RelocationEntry {
    /// Defer block xref.
    pub defer_xref: XrefId,
    /// Placeholder xref.
    pub placeholder_xref: Option<XrefId>,
    /// Loading xref.
    pub loading_xref: Option<XrefId>,
    /// Error xref.
    pub error_xref: Option<XrefId>,
}

/// Compilation result after all phases have run.
pub struct CompilationResult<'a> {
    /// The compiled template functions.
    pub template_fn: TemplateFn<'a>,
    /// Additional template functions for embedded views.
    pub embedded_fns: Vec<'a, TemplateFn<'a>>,
    /// Extracted constants.
    pub consts: Vec<'a, ConstValue<'a>>,
}

/// A compiled template function.
pub struct TemplateFn<'a> {
    /// Function name.
    pub name: Atom<'a>,
    /// Number of creation slots.
    pub creation_slots: u32,
    /// Number of variable slots.
    pub var_slots: u32,
    /// The function body statements.
    pub body: Vec<'a, FnStatement<'a>>,
}

// ============================================================================
// Host Binding Compilation
// ============================================================================

/// The kind of compilation job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompilationJobKind {
    /// Template compilation.
    Template,
    /// Host binding compilation.
    Host,
}

/// A compilation job for host bindings.
///
/// Host bindings are `@HostBinding()` and `@HostListener()` decorators
/// on a component or directive. They bind directly to the host element
/// rather than to template elements.
///
/// Ported from Angular's `HostBindingCompilationJob` in `compilation.ts`.
pub struct HostBindingCompilationJob<'a> {
    /// The allocator for this compilation.
    pub allocator: &'a Allocator,
    /// Name of the component/directive being compiled.
    pub component_name: Atom<'a>,
    /// The CSS selector for the component/directive.
    pub component_selector: Atom<'a>,
    /// Constant pool for deduplication.
    pub pool: ConstantPool<'a>,
    /// Expression store for managing expressions by reference.
    pub expressions: ExpressionStore<'a>,
    /// The root unit for host bindings.
    pub root: HostBindingCompilationUnit<'a>,
    /// Next cross-reference ID to allocate.
    next_xref_id: u32,
    /// Compatibility mode for output.
    pub compatibility_mode: CompatibilityMode,
    /// Template compilation mode (always DomOnly for host bindings).
    pub mode: TemplateCompilationMode,
    /// Function name suffix.
    pub fn_suffix: Atom<'a>,
    /// Diagnostics collected during compilation.
    pub diagnostics: std::vec::Vec<OxcDiagnostic>,
}

impl<'a> HostBindingCompilationJob<'a> {
    /// Creates a new host binding compilation job.
    pub fn new(
        allocator: &'a Allocator,
        component_name: Atom<'a>,
        component_selector: Atom<'a>,
    ) -> Self {
        Self::with_pool_starting_index(allocator, component_name, component_selector, 0)
    }

    /// Creates a new host binding compilation job with a specific constant pool starting index.
    ///
    /// This is used when compiling host bindings for a component that has already used
    /// some constant pool indices for template compilation. The host binding compilation
    /// continues from where the template compilation's pool left off to avoid duplicate
    /// constant names.
    ///
    /// For example, if template compilation uses _c0, _c1, _c2, then host binding
    /// compilation should be created with `pool_starting_index: 3` to start with _c3.
    ///
    /// This matches Angular TypeScript behavior where both template and host binding
    /// compilation share the same ConstantPool instance.
    pub fn with_pool_starting_index(
        allocator: &'a Allocator,
        component_name: Atom<'a>,
        component_selector: Atom<'a>,
        pool_starting_index: u32,
    ) -> Self {
        let root_xref = XrefId::new(0);
        let root = HostBindingCompilationUnit::new(allocator, root_xref);

        Self {
            allocator,
            component_name,
            component_selector,
            pool: ConstantPool::with_starting_index(allocator, pool_starting_index),
            expressions: ExpressionStore::new(allocator),
            root,
            next_xref_id: 1, // 0 is reserved for root
            compatibility_mode: CompatibilityMode::TemplateDefinitionBuilder,
            mode: TemplateCompilationMode::DomOnly, // Host bindings always use DomOnly
            fn_suffix: Atom::from("HostBindings"),
            diagnostics: std::vec::Vec::new(),
        }
    }

    /// Returns the kind of this compilation job.
    pub fn kind(&self) -> CompilationJobKind {
        CompilationJobKind::Host
    }

    /// Allocates a new cross-reference ID.
    pub fn allocate_xref_id(&mut self) -> XrefId {
        let id = XrefId::new(self.next_xref_id);
        self.next_xref_id += 1;
        id
    }

    /// Stores an expression and returns its ID.
    pub fn store_expression(
        &mut self,
        expr: crate::ast::expression::AngularExpression<'a>,
    ) -> super::expression_store::ExpressionId {
        self.expressions.store(expr)
    }

    /// Retrieves an expression by its ID.
    pub fn get_expression(
        &self,
        id: super::expression_store::ExpressionId,
    ) -> &crate::ast::expression::AngularExpression<'a> {
        self.expressions.get(id)
    }
}

/// A compilation unit for host bindings.
///
/// Unlike view compilation units, host binding units don't have embedded views
/// or child nodes. They only contain bindings that apply to the host element.
pub struct HostBindingCompilationUnit<'a> {
    /// Cross-reference ID for this unit (always 0 for host bindings).
    pub xref: XrefId,
    /// Create-time operations.
    pub create: CreateOpList<'a>,
    /// Update-time operations.
    pub update: UpdateOpList<'a>,
    /// Create-time statements (populated by reify phase).
    pub create_statements: Vec<'a, OutputStatement<'a>>,
    /// Update-time statements (populated by reify phase).
    pub update_statements: Vec<'a, OutputStatement<'a>>,
    /// Host attributes (collected during ingestion).
    ///
    /// These are static attributes like `class="host-class"` that
    /// need to be extracted to the component's hostAttrs.
    pub attributes: Option<crate::output::ast::OutputExpression<'a>>,
    /// Number of variable slots needed.
    pub vars: Option<u32>,
    /// Generated function name.
    pub fn_name: Option<Atom<'a>>,
}

impl<'a> HostBindingCompilationUnit<'a> {
    /// Creates a new host binding compilation unit.
    pub fn new(allocator: &'a Allocator, xref: XrefId) -> Self {
        Self {
            xref,
            create: CreateOpList::new(allocator),
            update: UpdateOpList::new(allocator),
            create_statements: Vec::new_in(allocator),
            update_statements: Vec::new_in(allocator),
            attributes: None,
            vars: None,
            fn_name: None,
        }
    }
}

/// A statement in a template function body.
#[derive(Debug)]
pub enum FnStatement<'a> {
    /// Creation-time block.
    CreationBlock(Vec<'a, FnStatement<'a>>),
    /// Update-time block.
    UpdateBlock(Vec<'a, FnStatement<'a>>),
    /// A runtime instruction call.
    Instruction(Instruction<'a>),
    /// A variable declaration.
    VarDecl(Atom<'a>),
}

/// A runtime instruction call.
#[derive(Debug)]
pub struct Instruction<'a> {
    /// Instruction name (e.g., "ɵɵelement").
    pub name: Atom<'a>,
    /// Arguments to the instruction.
    pub args: Vec<'a, InstructionArg<'a>>,
}

/// An argument to a runtime instruction.
#[derive(Debug)]
pub enum InstructionArg<'a> {
    /// A literal value.
    Literal(ConstValue<'a>),
    /// A reference to a const.
    ConstRef(u32),
    /// A slot reference.
    Slot(u32),
    /// An expression string.
    Expression(Atom<'a>),
}

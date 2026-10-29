//! Component metadata structures.
//!
//! This module defines the metadata extracted from `@Component` decorators.

use oxc_allocator::Vec;
use oxc_span::Span;
use oxc_str::Ident;

use super::dependency::R3DependencyMetadata;
use crate::directive::R3InputMetadata;
use crate::output::ast::OutputExpression;

/// Angular prerelease tag (`-next.N` / `-rc.N` in the published version).
///
/// Ordering follows Angular's release cadence: `next.N` builds come off
/// `main` weekly, `rc.N` builds are cut from the release branch later, so
/// `Next` < `Rc` < release. Unrecognized tags map to [`Prerelease::Other`],
/// which sorts below both so an unknown suffix is treated as earlier than
/// any published prerelease of the same `major.minor.patch`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Prerelease {
    /// Any prerelease tag that is not `next.N` or `rc.N`.
    Other,
    /// A `next.N` (main-branch snapshot) prerelease.
    Next(u32),
    /// An `rc.N` (release candidate) prerelease.
    Rc(u32),
}

impl Prerelease {
    /// Parse a prerelease tag like `next.0` or `rc.1`.
    ///
    /// Unknown tags are kept as [`Prerelease::Other`] rather than dropped so
    /// they still sort below the release of the same triple.
    pub fn parse(tag: &str) -> Self {
        let (channel, ordinal) = tag.split_once('.').unwrap_or((tag, "0"));
        let ordinal: u32 = ordinal.parse().unwrap_or(0);
        match channel {
            "next" => Self::Next(ordinal),
            "rc" => Self::Rc(ordinal),
            _ => Self::Other,
        }
    }
}

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
    /// Prerelease tag (e.g., `Next(0)` for `22.3.0-next.0`), if any.
    pub prerelease: Option<Prerelease>,
}

impl AngularVersion {
    /// Create a new AngularVersion for a stable release.
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch, prerelease: None }
    }

    /// Set the prerelease tag (e.g., `Prerelease::Next(0)` for `-next.0`).
    #[must_use]
    pub fn with_prerelease(mut self, prerelease: Prerelease) -> Self {
        self.prerelease = Some(prerelease);
        self
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

    /// Check if this version supports standalone `ɵɵinterpolate*` instructions (v20.0.0+).
    ///
    /// Angular v20 introduced standalone `ɵɵinterpolate1`–`ɵɵinterpolateV` instructions
    /// used as nested calls within `ɵɵproperty`/`ɵɵattribute`. Earlier versions use
    /// combined `ɵɵpropertyInterpolate*`/`ɵɵattributeInterpolate*` instructions.
    pub fn supports_value_interpolation(&self) -> bool {
        self.major >= 20
    }

    /// Check if this version supports `ɵɵdomProperty` (v20.0.0+).
    ///
    /// Angular v20 introduced `ɵɵdomProperty` for host/DomOnly property bindings.
    /// Earlier versions use `ɵɵhostProperty` instead.
    pub fn supports_dom_property(&self) -> bool {
        self.major >= 20
    }

    /// Check if this version supports the `@Service` decorator (v22.0.0+).
    ///
    /// Angular v22 introduced the `@Service` decorator as a lighter alternative to
    /// `@Injectable`. The runtime gains `compileService`/`ɵɵdefineService` in the same
    /// release, so earlier versions cannot consume downleveled `@Service` metadata.
    pub fn supports_service_decorator(&self) -> bool {
        self.major >= 22
    }

    /// Whether the default component change-detection strategy is `OnPush`
    /// (v22.0.0+).
    ///
    /// Angular v22 made `OnPush` (0) the default strategy and added `Eager` (1).
    /// The runtime computes `onPush = changeDetection !== Eager`, so the full
    /// definition omits `changeDetection` for `OnPush` and emits `1` for `Eager`.
    /// Before v22 the default was `Default`/`Eager` (1) and the runtime computed
    /// `onPush = changeDetection === OnPush`, so the definition instead omits the
    /// default and emits `0` for an explicit `OnPush`.
    pub fn change_detection_default_is_on_push(&self) -> bool {
        self.major >= 22
    }

    /// Check if this version emits control instructions at all (v21.2.0+).
    ///
    /// Angular's `specializeControlProperties` phase and the
    /// `ɵɵcontrolCreate()`/`ɵɵcontrol()` instructions were introduced in
    /// v21.2.0 for the `formField` control directive. Earlier versions never
    /// emit them.
    pub fn supports_control_instructions(&self) -> bool {
        self.major > 21 || (self.major == 21 && self.minor >= 2)
    }

    /// Check if this version emits control instructions for the extended set of
    /// control properties (v22.0.0+).
    ///
    /// Angular v22 broadened `specializeControlProperties` beyond `formField`:
    /// `formControl`, `formControlName`, and `ngModel` (including two-way
    /// `[(ngModel)]`) now also emit the paired `ɵɵcontrolCreate()` /
    /// `ɵɵcontrol()` instructions. Earlier versions only do this for `formField`,
    /// so emitting the extra instructions against a < v22 runtime would diverge.
    pub fn supports_extended_control_properties(&self) -> bool {
        self.major >= 22
    }

    /// Check if this version anchors `ɵɵcontrolCreate()` at the element's first
    /// create instruction (v22.2.0+).
    ///
    /// Through v22.1.x, `findCreateInstruction` kept the *last* matching create
    /// op, so `ɵɵcontrolCreate()` landed after `ElementEnd`/`ContainerEnd` (and
    /// `Template` was a valid anchor). v22.2.0 switched to the *first* matching
    /// op and narrowed the anchor kinds to `Element`, `ElementStart`,
    /// `Container`, and `ContainerStart`, so the instruction sits right after
    /// the element start (before listeners) and `ng-template` no longer gets
    /// control instructions.
    pub fn uses_first_control_anchor(&self) -> bool {
        self.major > 22 || (self.major == 22 && self.minor >= 2)
    }

    /// Check if this version uses modern optional-chaining semantics (v22.0.0+).
    ///
    /// Angular v22 changed the safe-navigation operator (`?.`) in template
    /// expressions to yield `undefined` (native optional chaining) instead of the
    /// legacy `null`. Earlier versions default to the legacy `== null ? null`
    /// expansion. Users can opt back into legacy behavior with the
    /// `legacyOptionalChaining` compiler option, or per-expression by wrapping it
    /// in the `$safeNavigationMigration(...)` magic function.
    ///
    /// See `angular/angular@2896c93cc1`.
    pub fn supports_modern_optional_chaining(&self) -> bool {
        self.major >= 22
    }

    /// Check if this version's runtime supports chained query instructions
    /// (`ɵɵviewQuery(p1)(p2)`, `ɵɵcontentQuerySignal(...)(...)`).
    ///
    /// Until `angular/angular@ae1c0dc4` ("perf(compiler): chain query creation
    /// instructions", landed in v21.1.0 and cherry-picked into v21.0.4 as
    /// `f901cc9e`), `ɵɵviewQuery` / `ɵɵcontentQuery` / `ɵɵviewQuerySignal` /
    /// `ɵɵcontentQuerySignal` all returned `void`. Chained emit on those
    /// versions throws at runtime: `TypeError: ɵɵviewQuery(...) is not a
    /// function`. From v21.0.4+ they return `typeof <fn>` and chain safely.
    pub fn supports_chained_queries(&self) -> bool {
        // v22+ unconditionally, v21.1.0+ on the v21 line, v21.0.4+ on the
        // v21.0 cherry-pick. Anything below v21 is `void`-return territory.
        if self.major >= 22 {
            return true;
        }
        if self.major == 21 {
            if self.minor >= 1 {
                return true;
            }
            return self.minor == 0 && self.patch >= 4;
        }
        false
    }

    /// Check if this version has the `ɵɵenableIncrementalHydrationRuntime`
    /// instruction (v22.0.0-next.11+).
    ///
    /// From that release `withIncrementalHydration()` alone no longer turns
    /// incremental hydration on: a compiled template must also call the
    /// instruction, so every view with a `hydrate` trigger emits it. Earlier
    /// runtimes don't export it and turn the feature on from the provider
    /// alone.
    pub fn supports_incremental_hydration_runtime(&self) -> bool {
        if self.major == 22 && self.minor == 0 && self.patch == 0 {
            return match self.prerelease {
                None => true,
                Some(prerelease) => prerelease >= Prerelease::Next(11),
            };
        }
        self.is_at_least(22, 0, 0)
    }

    /// Check if this version emits the `attrs` field on `ɵcmp` (before
    /// v22.0.0-next.4).
    ///
    /// Until then `compileComponentFromMetadata` pooled the attributes of the
    /// component's first selector (`[bitBadge]` gives `["bitBadge", ""]`) and set
    /// `attrs` to that constant. No runtime ever read it, and
    /// `angular/angular@d99ab0e040` ("fix(compiler): stop generating unused
    /// field") removed it in v22.0.0-next.4; it was never backported. The
    /// constant still takes a `_cN` index on older versions.
    pub fn emits_selector_attrs(&self) -> bool {
        if self.major == 22 && self.minor == 0 && self.patch == 0 {
            return matches!(self.prerelease, Some(prerelease) if prerelease < Prerelease::Next(4));
        }
        self.major < 22
    }

    /// Check if this version compiles component templates in DOM-only mode
    /// (v20.1.0-next.3+).
    ///
    /// From that release a full build compiles the template of a standalone
    /// component without directive dependencies with `ɵɵdomElementStart`,
    /// `ɵɵdomListener`, `ɵɵdomProperty` and the other DOM-only instructions
    /// (`TemplateCompilationMode.DomOnly`). `20.1.0-next.2` and earlier always
    /// use the full instructions.
    pub fn supports_dom_only_templates(&self) -> bool {
        if self.major == 20 && self.minor == 1 && self.patch == 0 {
            return match self.prerelease {
                None => true,
                Some(prerelease) => prerelease >= Prerelease::Next(3),
            };
        }
        self.is_at_least(20, 1, 0)
    }

    /// Check if this version's `ShadowCss` still runs the legacy Shadow DOM
    /// polyfill passes (before v22.0.0-rc.0).
    ///
    /// Until then `shimCssText` rewrote `polyfill-next-selector`,
    /// `polyfill-rule` and `polyfill-unscoped-rule` blocks and replaced the
    /// `::shadow` / `::content` combinators with a space. v22.0.0-rc.0 dropped
    /// all four passes (`22.0.0-next.12` still has them), so a v22 stylesheet
    /// scopes those tokens like any other selector text.
    pub fn uses_shadow_css_polyfills(&self) -> bool {
        if self.major == 22 && self.minor == 0 && self.patch == 0 {
            return matches!(self.prerelease, Some(prerelease) if prerelease < Prerelease::Rc(0));
        }
        self.major < 22
    }

    /// Check if this version is `major.minor.patch` or newer.
    ///
    /// Compares the release triple only: a prerelease of `22.3.0` counts as
    /// `>= 22.2.2` even though it predates the `22.3.0` stable. Callers gating
    /// on a fix that landed mid-`next` cycle must compare the stored
    /// prerelease against the cutoff explicitly (see
    /// [`AngularVersion::at_22_2_2_cutoff_or_later`]).
    pub fn is_at_least(&self, major: u32, minor: u32, patch: u32) -> bool {
        (self.major, self.minor, self.patch) >= (major, minor, patch)
    }

    /// True when the fixes shipped in `22.2.2` / `22.3.0-next.1` apply.
    ///
    /// Every prerelease of `22.3.0` before `next.1` — `next.0`, plus any
    /// unrecognized tag which parses to [`Prerelease::Other`] and sorts below
    /// `next`/`rc` — was cut before the fixes landed on `main`, so it must
    /// compare strictly below the `next.1` cutoff rather than be excluded by
    /// ordinal. [`AngularVersion::Rc`] sorts after `Next`, so `rc` builds
    /// take the fixed path.
    fn at_22_2_2_cutoff_or_later(&self) -> bool {
        if self.major == 22 && self.minor == 3 && self.patch == 0 {
            return match self.prerelease {
                None => true,
                Some(prerelease) => prerelease >= Prerelease::Next(1),
            };
        }
        self.is_at_least(22, 2, 2)
    }

    /// Check if this version drops `img|src` and `video|src` from the URL
    /// security context (v22.2.2+ / v22.3.0-next.1+).
    ///
    /// `angular/angular@439ff1bdf4` ("refactor(compiler): remove img/video src
    /// attribute from the URL security context") landed only on the v22 line,
    /// so a bound `src` on `<img>`/`<video>` no longer receives
    /// `ɵɵsanitizeUrl`. The 19.x/20.x/21.x security backports kept the keys.
    /// `22.3.0-next.0` and every earlier `22.3.0` prerelease are excluded:
    /// they sort after `22.2.2` by triple but were cut before the fix landed
    /// on `main` (first in `v22.3.0-next.1`).
    pub fn drops_img_video_src_url_context(&self) -> bool {
        self.at_22_2_2_cutoff_or_later()
    }

    /// Check if this version strips the `:svg:`/`:math:` namespace prefix from
    /// the single-root tag passed to `ɵɵconditionalCreate` /
    /// `ɵɵconditionalBranchCreate` / `ɵɵrepeaterCreate` (v22.2.2+ /
    /// v22.3.0-next.1+).
    ///
    /// `angular/angular@0244a1cb3` ("fix(compiler): match namespaced root nodes
    /// of control flow blocks against ng-content tag selectors") was never
    /// backported to the 21.x/20.x/19.x lines. `22.3.0-next.0` and every
    /// earlier `22.3.0` prerelease are excluded: they sort after `22.2.2` by
    /// triple but were cut before the fix landed on `main` (first in
    /// `v22.3.0-next.1`).
    pub fn strips_namespaced_block_tag(&self) -> bool {
        self.at_22_2_2_cutoff_or_later()
    }

    /// Check if this version sanitizes two-way property bindings
    /// (`[(innerHTML)]`, `[(href)]`, ...).
    ///
    /// `angular/angular@3c70270c96` added `TwoWayProperty` to
    /// `resolveSanitizers`. First released in 20.3.25, 21.2.17, 22.0.1 and
    /// 22.1.0-next.0; the 19.2.x line never got it.
    pub fn sanitizes_two_way_properties(&self) -> bool {
        self.fixed_since(&[
            Self::new(20, 3, 25),
            Self::new(21, 2, 17),
            Self::new(22, 0, 1),
            Self::new(22, 1, 0).with_prerelease(Prerelease::Next(0)),
        ])
    }

    /// Check if this version treats `href` / `xlink:href` of every MathML
    /// element as a URL sink (`:math:*`), not only a fixed element list.
    ///
    /// `angular/angular@3927a5b271`, first in 22.0.1 and 22.1.0-next.0. The
    /// 20.3.28 / 21.2.20 backports of the concrete-host fix
    /// (`angular/angular@2f96c8020f`, `angular/angular@6afe6fa781`) carry it
    /// too; the 19.2.x line never got it.
    pub fn matches_any_mathml_href(&self) -> bool {
        self.fixed_since(&[
            Self::new(20, 3, 28),
            Self::new(21, 2, 20),
            Self::new(22, 0, 1),
            Self::new(22, 1, 0).with_prerelease(Prerelease::Next(0)),
        ])
    }

    /// Check if this version's security lookup falls back from the
    /// `:svg:` / `:math:` table to the HTML table (`<svg><iframe [src]>` is a
    /// resource URL), and marks iframe `credentialless` as security
    /// sensitive.
    ///
    /// `angular/angular@32d7315094` and `angular/angular@0152e3cbdf`, both
    /// first in 22.0.2 and 22.1.0-next.1.
    pub fn uses_22_0_2_security_schema(&self) -> bool {
        self.fixed_since(&[
            Self::new(22, 0, 2),
            Self::new(22, 1, 0).with_prerelease(Prerelease::Next(1)),
        ])
    }

    /// Check if this version looks up an SVG animation element written
    /// outside `<svg>` (no namespace) in the SVG table.
    ///
    /// `angular/angular@7168bed663`, first in 22.1.5 and 22.2.0-next.5.
    pub fn validates_svg_animation_outside_svg(&self) -> bool {
        self.fixed_since(&[
            Self::new(22, 1, 5),
            Self::new(22, 2, 0).with_prerelease(Prerelease::Next(5)),
        ])
    }

    /// Check if this version computes host binding security contexts against
    /// every possible concrete host element instead of only the declaring
    /// selector.
    ///
    /// `angular/angular@d06e3748b7`, first in 20.3.28, 21.2.20, 22.1.0 and
    /// 22.2.0-next.0. No 22.0.x release has it.
    pub fn sanitizes_host_bindings_on_concrete_hosts(&self) -> bool {
        self.fixed_since(&[
            Self::new(20, 3, 28),
            Self::new(21, 2, 20),
            Self::new(22, 1, 0),
            Self::new(22, 2, 0).with_prerelease(Prerelease::Next(0)),
        ])
    }

    /// Whether a fix is in this version, given the first fixed release on
    /// each release line, oldest line first.
    ///
    /// A version on a listed `major.minor` line compares against that line's
    /// cutoff; a version newer than every listed line has the fix; a version
    /// on any other line does not.
    fn fixed_since(&self, cutoffs: &[AngularVersion]) -> bool {
        if let Some(cutoff) =
            cutoffs.iter().find(|c| c.major == self.major && c.minor == self.minor)
        {
            return self.cmp_release(cutoff).is_ge();
        }
        cutoffs.last().is_some_and(|newest| (self.major, self.minor) > (newest.major, newest.minor))
    }

    /// Orders two versions, a stable release sorting after its prereleases.
    fn cmp_release(&self, other: &AngularVersion) -> std::cmp::Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (self.prerelease, other.prerelease) {
                (None, None) => std::cmp::Ordering::Equal,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (Some(_), None) => std::cmp::Ordering::Less,
                (Some(a), Some(b)) => a.cmp(&b),
            })
    }

    /// Parse a version string like "19.0.0" or "19.0.0-rc.1".
    ///
    /// The `-next.N` / `-rc.N` prerelease tag is retained; any other suffix
    /// parses to [`Prerelease::Other`]. Returns `None` if the version string
    /// is invalid.
    pub fn parse(version_str: &str) -> Option<Self> {
        // Handle placeholder version (means latest/head)
        if version_str == "0.0.0-PLACEHOLDER" {
            return Some(Self::new(u32::MAX, 0, 0)); // Treat as latest
        }

        // "19.0.0-rc.1" -> base "19.0.0", prerelease tag "rc.1"
        let (base_version, prerelease) = match version_str.split_once('-') {
            Some((base, tag)) => (base, Some(Prerelease::parse(tag))),
            None => (version_str, None),
        };

        let mut parts = base_version.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);

        let mut version = Self::new(major, minor, patch);
        version.prerelease = prerelease;
        Some(version)
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
    /// Only check when inputs change or events are triggered. Angular v22's
    /// default strategy (`OnPush = 0`).
    #[default]
    OnPush,
    /// Check the component on every change detection cycle (`Eager = 1`). The
    /// member introduced in Angular v22 to replace `Default`.
    Eager,
    /// Check the component on every change detection cycle. The pre-v22 spelling
    /// (`Default = 1`), kept distinct from [`Eager`] so partial declarations
    /// preserve the exact `ChangeDetectionStrategy` member the author wrote —
    /// emitting `Eager` for an older Angular target would reference a member
    /// that does not exist there.
    ///
    /// [`Eager`]: ChangeDetectionStrategy::Eager
    Default,
}

/// Metadata extracted from an `@Component` decorator.
///
/// This represents all the configuration properties that can be
/// specified in the `@Component({...})` decorator.
#[derive(Debug)]
pub struct ComponentMetadata<'a> {
    /// The name of the component class.
    pub class_name: Ident<'a>,

    /// The span of the class declaration.
    pub class_span: Span,

    /// Where TypeScript puts the class name's `pos`: the end of the `class`
    /// keyword. `setClassDebugInfo`'s `lineNumber` is its line.
    pub class_name_pos: u32,

    /// The CSS selector for this component.
    pub selector: Option<Ident<'a>>,

    /// Inline template string.
    pub template: Option<Ident<'a>>,

    /// Where the inline template's text sits in the source file, when it is written as
    /// a single string or template literal. Used to point template errors at the file.
    pub template_span: Option<Span>,

    /// URL to an external template file.
    pub template_url: Option<Ident<'a>>,

    /// Inline styles array.
    pub styles: Vec<'a, Ident<'a>>,

    /// `false` when the decorator had a `styles` field the static evaluator
    /// could not resolve (e.g. an import `resolveImportedValues` does not
    /// cover): `styles` is then empty and must not be read as "no styles".
    /// `true` when `styles` was absent or fully evaluated.
    pub styles_resolved: bool,

    /// URLs to external stylesheet files.
    pub style_urls: Vec<'a, Ident<'a>>,

    /// Whether this is a standalone component.
    pub standalone: bool,

    /// View encapsulation mode.
    pub encapsulation: ViewEncapsulation,

    /// Change detection strategy, or `None` when the decorator does not specify
    /// one (so the runtime's version-dependent default applies).
    pub change_detection: Option<ChangeDetectionStrategy>,

    /// Host bindings and listeners.
    pub host: Option<HostMetadata<'a>>,

    /// Component imports (for standalone components).
    pub imports: Vec<'a, Ident<'a>>,

    /// The class `@defer` may lazy-load for each element of the `imports`
    /// array literal, by position: `X`, or `forwardRef(() => X)` with
    /// `@angular/core`'s `forwardRef` (which [`Self::imports`] leaves out),
    /// as ngtsc resolves them through its `forwardRefResolver`; `None` for
    /// anything else, such as `X as any`, which ngtsc keeps eager. Each class
    /// comes with the span of its identifier in the array.
    pub import_entry_classes: Vec<'a, Option<(Ident<'a>, Span)>>,

    /// The classes `imports` names, flattened the way ngtsc's partial
    /// evaluator resolves it without type information: through nested arrays,
    /// spreads and same-file `const` arrays (`imports: IMPORTS`), and with
    /// `ns.Cmp` members. Unlike [`Self::import_entry_classes`], these may come
    /// from outside the array literal, so `@defer` lists them in its blocks'
    /// dependency functions but lazy-loads only the classes that one of
    /// those entries names.
    pub import_refs: Vec<'a, ImportRef<'a>>,

    /// Exported names for template references.
    ///
    /// In Angular, `exportAs` can be a comma-separated string (e.g., "foo, bar"),
    /// which is split into an array and emitted as `exportAs: ["foo", "bar"]`.
    pub export_as: Vec<'a, Ident<'a>>,

    /// The decorator's `preserveWhitespaces`, or `None` when it does not set
    /// one and the project-wide `TransformOptions::preserve_whitespaces`
    /// applies.
    pub preserve_whitespaces: Option<bool>,

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
    pub outputs: Vec<'a, (Ident<'a>, Ident<'a>)>,

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
    pub external_styles: Vec<'a, Ident<'a>>,

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

    /// Local identifiers from the `@Component.deferredImports` array.
    ///
    /// Each entry is a class binding the user explicitly opted into
    /// lazy-loading via `@defer` blocks. The compiler wires them into the
    /// per-component deferrable-dependencies resolver function emitted as
    /// the third argument of `ɵɵdefer(...)`, and never includes them in the
    /// eager dependency surface.
    ///
    /// Mirrors Angular's `analysis.explicitlyDeferredTypes` populated by
    /// `collectExplicitlyDeferredSymbols` in
    /// `packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts`.
    /// Local compilation reads this field; full compilation additionally
    /// derives candidates from template usage of `imports: [...]`, which
    /// requires cross-file selector info OXC doesn't have.
    pub deferred_imports: Vec<'a, Ident<'a>>,

    /// `true` when the decorator provably brings nothing into the template's
    /// scope: `imports` and `deferredImports` are each absent or an empty
    /// array literal, and the config object has no spread that could add
    /// them. `false` when unknown.
    ///
    /// A standalone component whose scope is provably empty can only match
    /// itself in its template, which is what a full ngtsc build checks before
    /// compiling the template in DOM-only mode (`hasDirectiveDependencies` in
    /// `packages/compiler-cli/src/ngtsc/annotations/component/src/handler.ts`).
    pub imports_provably_empty: bool,

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
    pub schemas: Vec<'a, Ident<'a>>,

    /// Whether this component uses signal-based inputs.
    ///
    /// This is set to `true` only when the decorator explicitly contains `signals: true`.
    /// When true, the component definition should include `signals: true`.
    ///
    /// See: packages/compiler-cli/src/ngtsc/annotations/directive/src/shared.ts:382-390
    pub is_signal: bool,
}

/// A class an `@Component.imports` value names (see
/// [`ComponentMetadata::import_refs`]).
#[derive(Debug, Clone, Copy)]
pub struct ImportRef<'a> {
    /// The name `@defer` knows the reference by: `name`, or `ns.name`.
    pub key: Ident<'a>,
    /// The `import * as ns` binding of an `ns.name` member.
    pub namespace: Option<Ident<'a>>,
    /// The identifier that names the class.
    pub name: Ident<'a>,
    /// The span of that identifier. ngtsc emits a reference to an imported
    /// class as a `WrappedNodeExpr` of the identifier it was read from, so
    /// two references are equivalent only when they come from the same node.
    pub span: Span,
}

/// Metadata for a host directive.
///
/// Corresponds to TypeScript's `HostDirectiveDef` in the compiler.
/// See: packages/compiler/src/render3/view/compiler.ts:683-723
#[derive(Debug)]
pub struct HostDirectiveMetadata<'a> {
    /// The directive class name.
    pub directive: Ident<'a>,

    /// The source module of the directive (e.g., "@angular/common", "./my-directive").
    /// Used to generate proper namespace aliases for imported host directives.
    /// `None` for local directives defined in the same file.
    pub source_module: Option<Ident<'a>>,

    /// Input mappings: (publicName, internalName) pairs.
    /// Empty if no inputs are exposed.
    pub inputs: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Output mappings: (publicName, internalName) pairs.
    /// Empty if no outputs are exposed.
    pub outputs: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Whether this is a forward reference (requires wrapping in a function).
    pub is_forward_reference: bool,

    /// Whether `inputs` or `outputs` was written, even as an empty array.
    /// ngtsc keeps those as non-null mappings, so the entry is passed to
    /// `ɵɵHostDirectivesFeature` as `{ directive }` instead of the bare class.
    pub has_mapping_keys: bool,
}

impl<'a> HostDirectiveMetadata<'a> {
    /// Create a new HostDirectiveMetadata.
    pub fn new(allocator: &'a oxc_allocator::Allocator, directive: Ident<'a>) -> Self {
        Self {
            directive,
            source_module: None,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            is_forward_reference: false,
            has_mapping_keys: false,
        }
    }

    /// Check if this directive has input or output mappings.
    pub fn has_mappings(&self) -> bool {
        !self.inputs.is_empty() || !self.outputs.is_empty()
    }

    /// Set the source module for this host directive.
    pub fn with_source_module(mut self, source_module: Ident<'a>) -> Self {
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
    pub type_name: Ident<'a>,

    /// The source module of the dependency (e.g., "@angular/common", "./my-directive").
    /// Used to generate proper namespace aliases for imported dependencies.
    /// `None` for local dependencies defined in the same file.
    pub source_module: Option<Ident<'a>>,

    /// For directives: the CSS selector.
    pub selector: Option<Ident<'a>>,

    /// For directives: input property names.
    pub inputs: Vec<'a, Ident<'a>>,

    /// For directives: output property names.
    pub outputs: Vec<'a, Ident<'a>>,

    /// For directives: export names (exportAs).
    pub export_as: Vec<'a, Ident<'a>>,

    /// For directives: whether this is a component.
    pub is_component: bool,

    /// For pipes: the pipe name used in templates.
    pub pipe_name: Option<Ident<'a>>,

    /// Whether this is a forward reference.
    pub is_forward_reference: bool,
}

impl<'a> TemplateDependency<'a> {
    /// Create a new directive dependency.
    pub fn directive(
        allocator: &'a oxc_allocator::Allocator,
        type_name: Ident<'a>,
        selector: Ident<'a>,
        is_component: bool,
    ) -> Self {
        Self {
            kind: TemplateDependencyKind::Directive,
            type_name,
            source_module: None,
            selector: Some(selector),
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            export_as: Vec::new_in(&allocator),
            is_component,
            pipe_name: None,
            is_forward_reference: false,
        }
    }

    /// Create a new pipe dependency.
    pub fn pipe(
        allocator: &'a oxc_allocator::Allocator,
        type_name: Ident<'a>,
        pipe_name: Ident<'a>,
    ) -> Self {
        Self {
            kind: TemplateDependencyKind::Pipe,
            type_name,
            source_module: None,
            selector: None,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            export_as: Vec::new_in(&allocator),
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
    pub fn with_source_module(mut self, source_module: Ident<'a>) -> Self {
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
    pub properties: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Static host attributes: `{ 'role': 'button' }`, as the expressions
    /// `hostAttrs` holds (a string literal, or the expression ngtsc emits for
    /// a value it can't evaluate).
    pub attributes: Vec<'a, (Ident<'a>, OutputExpression<'a>)>,

    /// Host event listeners: `{ '(click)': 'onClick()' }`
    pub listeners: Vec<'a, (Ident<'a>, Ident<'a>)>,

    /// Special attribute for static class binding: `{ 'class': 'foo bar' }`
    /// Captured separately to generate class instructions during compilation.
    pub class_attr: Option<Ident<'a>>,

    /// Special attribute for static style binding: `{ 'style': 'color: red' }`
    /// Captured separately to generate style instructions during compilation.
    pub style_attr: Option<Ident<'a>>,
}

impl<'a> HostMetadata<'a> {
    /// Create a new empty HostMetadata.
    pub fn new(allocator: &'a oxc_allocator::Allocator) -> Self {
        Self {
            properties: Vec::new_in(&allocator),
            attributes: Vec::new_in(&allocator),
            listeners: Vec::new_in(&allocator),
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
        class_name: Ident<'a>,
        class_span: Span,
        implicit_standalone: bool,
    ) -> Self {
        Self {
            class_name,
            class_span,
            class_name_pos: class_span.start,
            selector: None,
            template: None,
            template_span: None,
            template_url: None,
            styles: Vec::new_in(&allocator),
            styles_resolved: true,
            style_urls: Vec::new_in(&allocator),
            standalone: implicit_standalone,
            encapsulation: ViewEncapsulation::default(),
            change_detection: None,
            host: None,
            imports: Vec::new_in(&allocator),
            import_entry_classes: Vec::new_in(&allocator),
            import_refs: Vec::new_in(&allocator),
            export_as: Vec::new_in(&allocator),
            preserve_whitespaces: None,
            constructor_deps: None,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            // Feature-related fields
            providers: None,
            view_providers: None,
            host_directives: Vec::new_in(&allocator),
            uses_inheritance: false,
            lifecycle: LifecycleMetadata::default(),
            external_styles: Vec::new_in(&allocator),
            // Template dependency fields
            declarations: Vec::new_in(&allocator),
            declaration_list_emit_mode: DeclarationListEmitMode::default(),
            raw_imports: None,
            deferred_imports: Vec::new_in(&allocator),
            imports_provably_empty: false,
            animations: None,
            schemas: Vec::new_in(&allocator),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_prerelease_tag() {
        assert_eq!(
            AngularVersion::parse("22.3.0-next.0"),
            Some(AngularVersion::new(22, 3, 0).with_prerelease(Prerelease::Next(0)))
        );
        assert_eq!(
            AngularVersion::parse("19.0.0-rc.1"),
            Some(AngularVersion::new(19, 0, 0).with_prerelease(Prerelease::Rc(1)))
        );
        // Unknown tags stay a prerelease (below the release) rather than being
        // dropped.
        assert_eq!(
            AngularVersion::parse("19.0.0-beta.2"),
            Some(AngularVersion::new(19, 0, 0).with_prerelease(Prerelease::Other))
        );
        assert_eq!(AngularVersion::parse("22.2.2"), Some(AngularVersion::new(22, 2, 2)));
        // Placeholder means latest/head: no prerelease, so every gate takes the
        // newest path.
        assert_eq!(
            AngularVersion::parse("0.0.0-PLACEHOLDER"),
            Some(AngularVersion::new(u32::MAX, 0, 0))
        );
    }

    /// Both 22.2.2 gates share the same cutoff, so they are exercised
    /// together: `>= 22.2.2` stable and `22.3.0-next.1+`, but not
    /// `22.3.0-next.0` or any earlier/unknown `22.3.0` prerelease — all of
    /// those were cut before the fixes landed on `main`.
    #[test]
    fn prerelease_gates_respect_22_3_0_next_1_cutoff() {
        let parse = |s| AngularVersion::parse(s).unwrap();

        for version in [
            parse("22.2.2"),
            parse("22.3.0-next.1"),
            parse("22.3.0-rc.0"),
            parse("22.3.0"),
            parse("23.0.0"),
            AngularVersion::new(u32::MAX, 0, 0),
        ] {
            assert!(version.drops_img_video_src_url_context(), "{version:?}");
            assert!(version.strips_namespaced_block_tag(), "{version:?}");
        }

        for version in [
            parse("22.2.1"),
            parse("22.3.0-next.0"),
            // Unknown tags parse to `Other` and sort below `next`/`rc`, so
            // hypothetical prereleases of 22.3.0 keep the old behavior.
            parse("22.3.0-beta.2"),
            parse("22.3.0-snapshot.9"),
            AngularVersion::new(22, 2, 0).with_prerelease(Prerelease::Next(5)),
        ] {
            assert!(!version.drops_img_video_src_url_context(), "{version:?}");
            assert!(!version.strips_namespaced_block_tag(), "{version:?}");
        }
    }

    /// Security fixes backported per release line: a version on a listed line
    /// compares against that line's first fixed release, a newer line has the
    /// fix, and an unlisted older line (21.0.x, 21.1.x, 22.0.x for host
    /// bindings) never got it.
    #[test]
    fn security_backports_follow_each_release_line() {
        let parse = |s| AngularVersion::parse(s).unwrap();

        for version in
            ["20.3.28", "21.2.20", "22.1.0", "22.1.3", "22.2.0-next.0", "22.2.2", "23.0.0"]
        {
            assert!(parse(version).sanitizes_host_bindings_on_concrete_hosts(), "{version}");
        }
        for version in [
            "19.2.25",
            "20.2.9",
            "20.3.27",
            "21.0.9",
            "21.2.19",
            "22.0.8",
            "22.1.0-rc.0",
            "22.2.0-beta.1",
        ] {
            assert!(!parse(version).sanitizes_host_bindings_on_concrete_hosts(), "{version}");
        }

        for version in ["20.3.25", "21.2.17", "22.0.1", "22.1.0-next.0", "22.1.0-rc.1"] {
            assert!(parse(version).sanitizes_two_way_properties(), "{version}");
        }
        for version in ["19.2.25", "20.3.24", "21.1.5", "21.2.16", "22.0.0", "22.1.0-beta.1"] {
            assert!(!parse(version).sanitizes_two_way_properties(), "{version}");
        }

        assert!(parse("22.1.0-next.1").uses_22_0_2_security_schema());
        assert!(!parse("22.1.0-next.0").uses_22_0_2_security_schema());
        for version in ["20.3.28", "21.2.20", "21.2.25", "22.0.1", "22.1.0-next.0"] {
            assert!(parse(version).matches_any_mathml_href(), "{version}");
        }
        for version in ["19.2.25", "20.3.27", "21.2.19", "22.0.0"] {
            assert!(!parse(version).matches_any_mathml_href(), "{version}");
        }
        assert!(parse("22.2.0-next.5").validates_svg_animation_outside_svg());
        assert!(!parse("22.2.0-next.4").validates_svg_animation_outside_svg());
        assert!(!parse("22.0.8").validates_svg_animation_outside_svg());
    }

    #[test]
    fn incremental_hydration_runtime_starts_at_22_0_0_next_11() {
        let parse = |s| AngularVersion::parse(s).unwrap();

        for version in [
            parse("22.0.0-next.11"),
            parse("22.0.0-rc.0"),
            parse("22.0.0"),
            parse("22.1.0-next.0"),
            parse("23.0.0"),
            AngularVersion::new(u32::MAX, 0, 0),
        ] {
            assert!(version.supports_incremental_hydration_runtime(), "{version:?}");
        }

        for version in [
            parse("21.2.25"),
            parse("22.0.0-next.0"),
            parse("22.0.0-next.10"),
            // Unknown tags parse to `Other`, below `next`.
            parse("22.0.0-beta.1"),
        ] {
            assert!(!version.supports_incremental_hydration_runtime(), "{version:?}");
        }
    }

    #[test]
    fn dom_only_templates_start_at_20_1_0_next_3() {
        let parse = |s| AngularVersion::parse(s).unwrap();

        for version in [
            parse("20.1.0-next.3"),
            parse("20.1.0-rc.0"),
            parse("20.1.0"),
            parse("20.3.4"),
            parse("21.0.0-next.0"),
            parse("22.2.2"),
        ] {
            assert!(version.supports_dom_only_templates(), "{version:?}");
        }

        for version in [
            parse("19.2.0"),
            parse("20.0.7"),
            parse("20.1.0-next.2"),
            // Unknown tags parse to `Other`, below `next`.
            parse("20.1.0-beta.1"),
        ] {
            assert!(!version.supports_dom_only_templates(), "{version:?}");
        }
    }

    #[test]
    fn shadow_css_polyfills_end_at_22_0_0_rc_0() {
        let parse = |s| AngularVersion::parse(s).unwrap();

        for version in [
            parse("19.2.25"),
            parse("20.3.18"),
            parse("21.2.25"),
            parse("22.0.0-next.12"),
            // Unknown tags parse to `Other`, below `next`.
            parse("22.0.0-beta.1"),
        ] {
            assert!(version.uses_shadow_css_polyfills(), "{version:?}");
        }

        for version in [
            parse("22.0.0-rc.0"),
            parse("22.0.0"),
            parse("22.2.2"),
            parse("23.0.0"),
            AngularVersion::new(u32::MAX, 0, 0),
        ] {
            assert!(!version.uses_shadow_css_polyfills(), "{version:?}");
        }
    }

    #[test]
    fn selector_attrs_end_at_22_0_0_next_4() {
        let parse = |s| AngularVersion::parse(s).unwrap();

        for version in
            [parse("19.2.20"), parse("20.3.34"), parse("21.2.25"), parse("22.0.0-next.3")]
        {
            assert!(version.emits_selector_attrs(), "{version:?}");
        }

        for version in [
            parse("22.0.0-next.4"),
            parse("22.0.0-rc.0"),
            parse("22.0.0"),
            parse("22.2.2"),
            AngularVersion::new(u32::MAX, 0, 0),
        ] {
            assert!(!version.emits_selector_attrs(), "{version:?}");
        }
    }
}

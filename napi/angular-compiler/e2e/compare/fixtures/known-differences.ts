/**
 * Fixtures whose output differs from Angular's in documented ways, keyed by
 * `category/name`. Each entry lists every difference and why it exists.
 *
 * A listed fixture is reported as a known difference only while it differs in exactly what the
 * entry (and `GLOBAL_KNOWN_DIFFERENCES`) lists: a difference anywhere else (another static field,
 * class, top-level function or constant, export, import, metadata call or top-level statement)
 * still fails, and so does a listed difference that no longer occurs, so fixed entries get
 * removed.
 */
import type { ImportDiff } from '../src/compare.js'

export interface KnownDifference {
  /** `Class.field` of every static field that differs, e.g. `MyComponent.ɵcmp`. */
  fields: string[]
  /** Every class whose declaration (apart from its `static ɵxxx` fields) differs. */
  classes?: string[]
  /** Every top-level function or constant that differs, is missing or is extra. */
  functions?: string[]
  /** Every export that is missing, extra or points at another binding. */
  exports?: string[]
  /** Every import difference, exactly as `compareImports` reports it. */
  importDiffs?: ImportDiff[]
  /** Every setClassMetadata difference, as `Class.field`, e.g. `MyComponent.setClassMetadata`. */
  metadataDiffs?: string[]
  /**
   * Every other top-level statement that differs, by `StatementDiff.label`, e.g.
   * `ɵɵsetNgModuleScope(AppModule)`.
   */
  statements?: string[]
  /** Why each difference exists. */
  reasons: string[]
}

const FACTORY =
  'ɵfac uses ɵɵgetInheritedFactory where Angular emits a factory that calls the constructor'
const NG_MODULE_IMPORTS = 'ɵmod includes imports, which Angular omits in full compilation'
const HOST_ATTRS_ORDER = 'hostAttrs are emitted in a different order'
const CSS_WS = 'runs of whitespace inside shimmed selectors are collapsed where Angular keeps them'
const DEBUG_NAME =
  'signal(), input() and the other signal APIs lack the ngDevMode `debugName` argument ngtsc adds to class fields'
const DOM_ONLY =
  'templates use ɵɵelementStart/ɵɵlistener where a full ngtsc build of a standalone component without directive dependencies emits the DOM-only ɵɵdomElementStart/ɵɵdomListener'
const COMPONENT_DEPS =
  'dependencies use the local-compilation ɵɵgetComponentDepsFactory(Cmp, imports) where a full ngtsc build lists the resolved dependencies'
const INJECTABLE_FACTORY_WRAPPER =
  'an @Injectable useFactory is wrapped in a function expression where Angular emits an arrow function'
const SET_CLASS_METADATA_IMPORT =
  'Oxc keeps the @Inject import because its setClassMetadata references it; ngtsc emits the same reference but TypeScript elision still drops the import (upstream emit bug)'
const ALIASED_INJECTABLE =
  'an aliased @Injectable compiles to a broken ɵprov under ngtsc — its needsFactory check compares the written decorator name, so the factory (and setClassMetadata) are never emitted; Oxc emits a working ɵfac (issue #507, kept deliberately)'

const NG_MODULE_SCOPE =
  'the ngJitMode-guarded ɵɵsetNgModuleScope call Angular emits after an NgModule is missing'
const DEBUG_INFO_PATH =
  'ɵsetClassDebugInfo filePath is the path Oxc was given (absolute here and in the Vite plugin) where Angular emits it relative to the project root'
const DEBUG_INFO_LINE =
  'ɵsetClassDebugInfo lineNumber counts only \\n line breaks where TypeScript also counts \\r, U+2028 and U+2029'
const DEBUG_INFO_ORDER =
  'ɵsetClassDebugInfo is emitted before ɵsetClassMetadata where Angular emits it after'

/**
 * Differences every fixture may have, matched against the labels `KnownDifference` uses. A
 * fixture whose differences are all documented here or in its own entry is reported as a known
 * difference with these reasons. They never go stale.
 */
export const GLOBAL_KNOWN_DIFFERENCES: Array<{ label: RegExp; reason: string }> = [
  { label: /^statement ɵsetClassDebugInfo\([^()]+\)(#\d+)? filePath$/, reason: DEBUG_INFO_PATH },
  { label: /^statement order ɵsetClassDebugInfo\([^()]+\)(#\d+)?$/, reason: DEBUG_INFO_ORDER },
]

export const KNOWN_DIFFERENCES: Record<string, KnownDifference> = {
  'animations/animation-metadata-with-change-detection': {
    fields: ['AnimationsWithOnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY, COMPONENT_DEPS],
  },
  'class-metadata/class-metadata-ngmodule': {
    fields: ['TestableModule.ɵfac', 'TestableModule.ɵmod'],
    statements: ['ɵɵsetNgModuleScope(TestableModule)'],
    reasons: [FACTORY, NG_MODULE_IMPORTS, NG_MODULE_SCOPE],
  },
  'class-metadata/class-metadata-pipe': {
    fields: ['TestablePipe.ɵfac'],
    reasons: [FACTORY],
  },
  'class-metadata/class-metadata-injectable': {
    fields: [],
    importDiffs: [
      {
        type: 'different',
        moduleSource: '@angular/core',
        expected: ['Injectable', 'InjectionToken'],
        actual: ['Inject', 'Injectable', 'InjectionToken'],
      },
    ],
    reasons: [SET_CLASS_METADATA_IMPORT],
  },
  'class-metadata/class-metadata-with-inject': {
    fields: [],
    importDiffs: [
      {
        type: 'different',
        moduleSource: '@angular/core',
        expected: ['Component', 'InjectionToken'],
        actual: ['Component', 'Inject', 'InjectionToken'],
      },
    ],
    reasons: [SET_CLASS_METADATA_IMPORT],
  },
  'full-file/component-with-services': {
    fields: [],
    importDiffs: [
      {
        type: 'different',
        moduleSource: '@angular/core',
        expected: ['Component', 'InjectionToken', 'inject'],
        actual: ['Component', 'Inject', 'InjectionToken', 'inject'],
      },
    ],
    reasons: [SET_CLASS_METADATA_IMPORT],
  },
  'component-meta/change-detection-default': {
    fields: ['ChangeDetectionDefaultComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'component-meta/change-detection-onpush': {
    fields: ['ChangeDetectionOnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'edge-cases/aliased-injectable': {
    fields: ['AliasedService.ɵfac'],
    metadataDiffs: ['AliasedService.setClassMetadata'],
    reasons: [ALIASED_INJECTABLE],
  },
  'edge-cases/unicode-special-escapes': {
    fields: [],
    statements: ['ɵsetClassDebugInfo(UnicodeSpecialEscapesComponent)'],
    reasons: [DEBUG_INFO_LINE, DEBUG_INFO_PATH],
  },
  'edge-cases/multiple-custom-decorators': {
    fields: ['MultiDecoratorComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'edge-cases/single-custom-decorator': {
    fields: ['MyComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'full-file/component-with-pipes': {
    fields: ['ComponentWithPipes.ɵcmp'],
    reasons: ['component styles keep a leading newline'],
  },
  'full-transform/full-transform-complex': {
    fields: [],
    functions: [
      'ComplexComponent_Conditional_0_Template',
      'ComplexComponent_Conditional_1_For_1_Template',
    ],
    reasons: [DOM_ONLY],
  },
  'full-transform/full-transform-host-attribute': {
    fields: ['HostAttributeComponent.ɵcmp'],
    reasons: [HOST_ATTRS_ORDER],
  },
  'full-transform/full-transform-onpush': {
    fields: ['OnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'full-transform/transform-api-onpush': {
    fields: ['OnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'host-bindings/host-attribute': {
    fields: ['HostAttributeComponent.ɵcmp'],
    reasons: [HOST_ATTRS_ORDER],
  },
  'host-directives/host-directives-empty-mappings': {
    fields: ['EmptyMappingsComponent.ɵcmp'],
    reasons: [
      'HostDirectivesFeature receives the directive class where Angular passes { directive }',
    ],
  },
  'host-directives/host-directives-with-change-detection': {
    fields: ['HostDirectivesOnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY, COMPONENT_DEPS],
  },
  'host-directives/host-directives-with-providers': {
    fields: ['DataService.ɵfac'],
    reasons: [FACTORY],
  },
  'i18n/i18n-compliance-element-attributes-should-correctly-bind-to-context-in-nested-template-interpolation-nested-context':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-element-attributes-should-support-i18n-attributes-with-interpolations-on-explicit-ng-template-elements-with-structural-directives-ng-t':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-element-attributes-should-support-interpolation-interpolation-basic': {
    fields: ['UppercasePipe.ɵfac'],
    reasons: [FACTORY],
  },
  'i18n/i18n-compliance-nested-nodes-should-handle-i18n-attributes-with-bindings-and-nested-elements-in-content-nested-elements':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-nested-nodes-should-handle-i18n-attributes-with-bindings-in-content-and-element-attributes-nested-elements-with-i18n-attributes':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-nested-nodes-should-handle-i18n-attributes-with-bindings-in-content-bindings-in-content':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-nested-nodes-should-support-interpolations-with-complex-expressions-interpolation-complex-expressions':
    {
      fields: ['AsyncPipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-ng-container-ng-template-should-be-able-to-act-as-child-elements-inside-i18n-block-child-elements':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-ng-container-ng-template-should-handle-single-translation-message-using-ng-container-single-ng-container':
    {
      fields: ['UppercasePipe.ɵfac'],
      reasons: [FACTORY],
    },
  'i18n/i18n-compliance-root-should-support-i18n-message-with-multiple-pipes-multiple-pipes': {
    fields: ['PipeA.ɵfac', 'PipeB.ɵfac', 'PipeC.ɵfac'],
    reasons: [FACTORY],
  },
  'i18n/i18n-ng-template-deeply-nested': {
    fields: ['ActorTextPipe.ɵfac', 'UsernamePipe.ɵfac'],
    reasons: [FACTORY],
  },
  'injector/basic-injector': {
    fields: ['AppModule.ɵfac'],
    reasons: [FACTORY],
  },
  'injector/injector-with-both': {
    fields: ['FullModule.ɵfac', 'FullModule.ɵmod'],
    statements: ['ɵɵsetNgModuleScope(FullModule)'],
    reasons: [FACTORY, NG_MODULE_IMPORTS, NG_MODULE_SCOPE],
  },
  'injector/injector-with-imports': {
    fields: ['FeatureModule.ɵfac', 'FeatureModule.ɵmod'],
    statements: ['ɵɵsetNgModuleScope(FeatureModule)'],
    reasons: [FACTORY, NG_MODULE_IMPORTS, NG_MODULE_SCOPE],
  },
  'injector/injector-with-providers': {
    fields: ['ProvidersModule.ɵfac'],
    reasons: [FACTORY],
  },
  'pipe-compilation/impure-pipe': {
    fields: ['MyImpurePipe.ɵfac'],
    reasons: [FACTORY],
  },
  'pipe-compilation/minimal-pipe': {
    fields: ['MinimalPipe.ɵfac'],
    reasons: [FACTORY],
  },
  'pipe-compilation/non-standalone-pipe': {
    fields: ['LegacyPipe.ɵfac'],
    reasons: [FACTORY],
  },
  'pipe-compilation/pure-pipe': {
    fields: ['MyPurePipe.ɵfac'],
    reasons: [FACTORY],
  },
  'providers/object-members-injectable-use-factory': {
    fields: ['ObjectMembersService.ɵfac', 'ObjectMembersService.ɵprov'],
    reasons: [FACTORY, INJECTABLE_FACTORY_WRAPPER],
  },
  'providers/object-members-injectable-use-value': {
    fields: ['ObjectMembersValueService.ɵfac'],
    reasons: [FACTORY],
  },
  'providers/object-members-ng-module': {
    fields: ['ObjectMembersModule.ɵfac'],
    reasons: [FACTORY],
  },
  'providers/object-members-options-directive-pipe-module': {
    fields: ['OptionsPipe.ɵfac', 'OptionsModule.ɵfac'],
    reasons: [FACTORY],
  },
  'providers/object-members-options-injectable': {
    fields: ['OptionsMethodService.ɵfac', 'OptionsGetterService.ɵfac'],
    reasons: [FACTORY],
  },
  'providers/providers-with-change-detection': {
    fields: ['ProvidersWithChangeDetectionComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'regressions/class-debug-info-change-detection': {
    fields: ['OnPushDebugInfoComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'regressions/formfield-alias-repro': {
    fields: ['AppComponent.ɵcmp'],
    classes: ['AppComponent'],
    reasons: [COMPONENT_DEPS, DEBUG_NAME],
  },
  'schemas/schemas-with-change-detection': {
    fields: ['SchemasWithOnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'styles/shadow-css-child-combinator': {
    fields: ['ChildCombinatorComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-deep-combinator': {
    fields: ['DeepCombinatorComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-descendant-selectors': {
    fields: ['DescendantSelectorComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-double-where-host-context': {
    fields: ['DoubleWhereHostContextComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-has-basic': {
    fields: ['HasBasicComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-has-with-host': {
    fields: ['HasWithHostComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-attribute': {
    fields: ['HostContextAttributeComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-basic': {
    fields: ['HostContextBasicComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-comma-child': {
    fields: ['HostContextCommaChildComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-comma-list': {
    fields: ['HostContextCommaListComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-double': {
    fields: ['HostContextDoubleComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-host-different': {
    fields: ['HostContextHostDifferentComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-host-same-element': {
    fields: ['HostContextHostSameComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-nested-pseudo': {
    fields: ['HostContextNestedPseudoComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-no-selector': {
    fields: ['HostContextNoSelectorComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-pseudo-not': {
    fields: ['HostContextPseudoNotComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-tag': {
    fields: ['HostContextTagComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-context-triple': {
    fields: ['HostContextTripleComponent.ɵcmp'],
    reasons: [':host-context selector permutations are in a different order'],
  },
  'styles/shadow-css-host-context-with-descendant': {
    fields: ['HostContextDescendantComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-host-context': {
    fields: ['HostHostContextComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-host-with-descendant': {
    fields: ['HostDescendantComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-is-with-host': {
    fields: ['IsWithHostComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-mixed-host-in-where': {
    fields: ['MixedHostWhereComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-multiline-selector': {
    fields: ['MultilineSelectorComponent.ɵcmp'],
    reasons: ['the newline between selectors in a list is dropped'],
  },
  'styles/shadow-css-ng-deep-after-selector': {
    fields: ['NgDeepAfterSelectorComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-ng-deep-basic': {
    fields: ['NgDeepBasicComponent.ɵcmp'],
    reasons: ['a leading space is kept before a ::ng-deep selector'],
  },
  'styles/shadow-css-ng-deep-with-host': {
    fields: ['NgDeepWithHostComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-polyfill-next-selector': {
    fields: ['PolyfillNextSelectorComponent.ɵcmp'],
    reasons: ['a space is added before an empty rule body'],
  },
  'styles/shadow-css-polyfill-rule': {
    fields: ['PolyfillRuleComponent.ɵcmp'],
    reasons: ['polyfill-rule output drops a leading ";" from the rule body'],
  },
  'styles/shadow-css-polyfill-unscoped-rule': {
    fields: ['PolyfillUnscopedComponent.ɵcmp'],
    reasons: ['polyfill-unscoped-rule is not scoped and rewritten as Angular does'],
  },
  'styles/shadow-css-shadow-piercing': {
    fields: ['ShadowPiercingComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-sibling-combinators': {
    fields: ['SiblingCombinatorComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-where-basic': {
    fields: ['WhereBasicComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-where-host-context': {
    fields: ['WhereHostContextComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-where-nested': {
    fields: ['WhereNestedComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/shadow-css-where-with-host': {
    fields: ['WhereWithHostComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'styles/style-onpush-encapsulation': {
    fields: ['OnPushStyleComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'styles/style-onpush-none': {
    fields: ['OnPushNoneStyleComponent.ɵcmp'],
    reasons: [DOM_ONLY],
  },
  'templates/arrow-control-flow-expressions': {
    fields: [],
    classes: ['ArrowControlFlowExpressionsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-i18n': {
    fields: [],
    classes: ['ArrowHoistedI18nComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-if-alias': {
    fields: [],
    classes: ['ArrowHoistedIfAliasComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-nested-views': {
    fields: [],
    classes: ['ArrowHoistedNestedViewsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-ng-template-variables': {
    fields: [],
    classes: ['ArrowHoistedNgTemplateVariablesComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-order': {
    fields: [],
    classes: ['ArrowHoistedOrderComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-root-reference': {
    fields: [],
    classes: ['ArrowHoistedRootReferenceComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-same-arrow-in-two-views': {
    fields: [],
    classes: ['ArrowHoistedSameArrowInTwoViewsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-hoisted-two-in-one-view': {
    fields: [],
    classes: ['ArrowHoistedTwoInOneViewComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-interpolation-identical-arrows': {
    fields: [],
    classes: ['ArrowInterpolationIdenticalArrowsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-interpolation-multiple-params': {
    fields: [],
    classes: ['ArrowInterpolationMultipleParamsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-interpolation-no-param': {
    fields: [],
    classes: ['ArrowInterpolationNoParamComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-interpolation-one-param': {
    fields: [],
    classes: ['ArrowInterpolationOneParamComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-interpolation-param-and-property': {
    fields: [],
    classes: ['ArrowInterpolationParamAndPropertyComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-let-chain': {
    fields: [],
    classes: ['ArrowLetChainComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-let-in-for': {
    fields: [],
    classes: ['ArrowLetInForComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-let-reading-let': {
    fields: [],
    classes: ['ArrowLetReadingLetComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-multiple-params': {
    fields: [],
    classes: ['ArrowListenerMultipleParamsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-nested': {
    fields: [],
    classes: ['ArrowListenerNestedComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-no-param': {
    fields: [],
    classes: ['ArrowListenerNoParamComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-one-param': {
    fields: [],
    classes: ['ArrowListenerOneParamComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-param-and-property': {
    fields: [],
    classes: ['ArrowListenerParamAndPropertyComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-param-shadows-for-item': {
    fields: [],
    classes: ['ArrowListenerParamShadowsForItemComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-param-shadows-let': {
    fields: [],
    classes: ['ArrowListenerParamShadowsLetComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-param-shadows-property': {
    fields: [],
    classes: ['ArrowListenerParamShadowsPropertyComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-param-shadows-ref': {
    fields: [],
    classes: ['ArrowListenerParamShadowsRefComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-listener-reads-template-variables': {
    fields: [],
    classes: ['ArrowListenerReadsTemplateVariablesComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-nested': {
    fields: [],
    classes: ['ArrowPropertyNestedComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-no-param': {
    fields: [],
    classes: ['ArrowPropertyNoParamComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-one-param': {
    fields: [],
    classes: ['ArrowPropertyOneParamComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-param-and-property': {
    fields: [],
    classes: ['ArrowPropertyParamAndPropertyComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-param-shadows-for-item': {
    fields: [],
    classes: ['ArrowPropertyParamShadowsForItemComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-param-shadows-let': {
    fields: [],
    classes: ['ArrowPropertyParamShadowsLetComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-param-shadows-property': {
    fields: [],
    classes: ['ArrowPropertyParamShadowsPropertyComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-param-shadows-ref': {
    fields: [],
    classes: ['ArrowPropertyParamShadowsRefComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-reads-template-variables': {
    fields: [],
    classes: ['ArrowPropertyReadsTemplateVariablesComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/arrow-property-several-bindings': {
    fields: [],
    classes: ['ArrowPropertySeveralBindingsComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/case-matches-component': {
    fields: [],
    classes: ['ViewCmp'],
    reasons: [DEBUG_NAME],
  },
  'templates/case-raw-text': {
    fields: ['CaseRawTextComponent.ɵcmp'],
    reasons: [CSS_WS],
  },
  'templates/ng-arrow-function-dollar-event': {
    fields: [],
    classes: ['NgArrowFunctionDollarEventComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/ng-arrow-function-host-listener': {
    fields: [],
    classes: ['TestDir'],
    reasons: [DEBUG_NAME],
  },
  'templates/ng-arrow-function-nested-listeners': {
    fields: [],
    classes: ['NgArrowFunctionNestedListenersComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/ng-arrow-function-no-context': {
    fields: [],
    classes: ['NgArrowFunctionNoContextComponent'],
    reasons: [DEBUG_NAME],
  },
  'templates/ng-arrow-function-pipe': {
    fields: ['TestPipe.ɵfac'],
    reasons: [FACTORY],
  },
}

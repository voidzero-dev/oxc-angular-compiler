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

const DEBUG_NAME =
  'signal(), input() and the other signal APIs lack the ngDevMode `debugName` argument ngtsc adds to class fields'
const DOM_ONLY =
  'templates use ɵɵelementStart/ɵɵlistener where a full ngtsc build of a standalone component without directive dependencies emits the DOM-only ɵɵdomElementStart/ɵɵdomListener'
const COMPONENT_DEPS =
  'dependencies use the local-compilation ɵɵgetComponentDepsFactory(Cmp, imports) where a full ngtsc build lists the resolved dependencies'
const SELF_DEPENDENCY =
  'a standalone component without imports that uses its own selector in its template has no `dependencies` where a full ngtsc build lists the component itself (ngtsc local mode omits it too)'
const SET_CLASS_METADATA_IMPORT =
  'Oxc keeps the @Inject import because its setClassMetadata references it; ngtsc emits the same reference but TypeScript elision still drops the import (upstream emit bug)'
const ALIASED_INJECTABLE =
  'an aliased @Injectable compiles to a broken ɵprov under ngtsc — its needsFactory check compares the written decorator name, so the factory (and setClassMetadata) are never emitted; Oxc emits a working ɵfac (issue #507, kept deliberately)'

const NG_MODULE_SCOPE =
  'ɵɵsetNgModuleScope runs unguarded where Angular wraps it in `ngJitMode`: Oxc compiles one file at a time and resolves component dependencies at runtime through the scope it registers, so production builds (ngJitMode = false) still need it'
const DEBUG_INFO_PATH =
  'ɵsetClassDebugInfo filePath is the path Oxc was given (absolute here and in the Vite plugin) where Angular emits it relative to the project root'

/**
 * Differences every fixture may have, matched against the labels `KnownDifference` uses. A
 * fixture whose differences are all documented here or in its own entry is reported as a known
 * difference with these reasons. They never go stale.
 */
export const GLOBAL_KNOWN_DIFFERENCES: Array<{ label: RegExp; reason: string }> = [
  { label: /^statement ɵsetClassDebugInfo\([^()]+\)(#\d+)? filePath$/, reason: DEBUG_INFO_PATH },
]

export const KNOWN_DIFFERENCES: Record<string, KnownDifference> = {
  'class-metadata/class-metadata-ngmodule': {
    fields: [],
    statements: ['ɵɵsetNgModuleScope(TestableModule)'],
    reasons: [NG_MODULE_SCOPE],
  },
  'injector/injector-with-both': {
    fields: [],
    statements: ['ɵɵsetNgModuleScope(FullModule)'],
    reasons: [NG_MODULE_SCOPE],
  },
  'injector/injector-with-imports': {
    fields: [],
    statements: ['ɵɵsetNgModuleScope(FeatureModule)'],
    reasons: [NG_MODULE_SCOPE],
  },
  'injector/injector-with-spread-declarations': {
    fields: [],
    statements: ['ɵɵsetNgModuleScope(SpreadModule)'],
    reasons: [NG_MODULE_SCOPE],
  },
  'injector/injector-with-exports': {
    fields: [],
    statements: ['ɵɵsetNgModuleScope(ExportingModule)'],
    reasons: [NG_MODULE_SCOPE],
  },
  'animations/animation-metadata-with-change-detection': {
    fields: ['AnimationsWithOnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY, COMPONENT_DEPS],
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
  'factory/component-inject-string-token': {
    fields: [],
    importDiffs: [
      {
        type: 'different',
        moduleSource: '@angular/core',
        expected: ['Component'],
        actual: ['Component', 'Inject', 'Optional', 'Self'],
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
  'edge-cases/aliased-injectable': {
    fields: ['AliasedService.ɵfac'],
    metadataDiffs: ['AliasedService.setClassMetadata'],
    reasons: [ALIASED_INJECTABLE],
  },
  'edge-cases/forwardref-basic-import': {
    fields: ['ForwardRefBasicImportComponent.ɵcmp'],
    reasons: [COMPONENT_DEPS],
  },
  'edge-cases/forwardref-mixed-imports': {
    fields: ['ForwardRefMixedImportsComponent.ɵcmp'],
    reasons: [COMPONENT_DEPS],
  },
  'edge-cases/forwardref-multiple-imports': {
    fields: ['ForwardRefMultipleImportsComponent.ɵcmp'],
    reasons: [COMPONENT_DEPS],
  },
  'edge-cases/forwardref-with-template': {
    fields: ['ForwardRefWithTemplateComponent.ɵcmp'],
    reasons: [COMPONENT_DEPS],
  },
  'host-directives/host-directives-with-change-detection': {
    fields: ['HostDirectivesOnPushComponent.ɵcmp'],
    reasons: [DOM_ONLY, COMPONENT_DEPS],
  },
  'regressions/formfield-alias-repro': {
    fields: ['AppComponent.ɵcmp'],
    classes: ['AppComponent'],
    reasons: [COMPONENT_DEPS, DEBUG_NAME],
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
  'templates/dom-only-empty-imports': {
    fields: ['DomOnlyEmptyImportsComponent.ɵcmp'],
    reasons: [COMPONENT_DEPS],
  },
  'templates/dom-only-not-for-default-selector-self-reference': {
    fields: ['DefaultSelectorComponent.ɵcmp'],
    reasons: [SELF_DEPENDENCY],
  },
  'templates/dom-only-not-for-self-reference': {
    fields: ['TreeComponent.ɵcmp'],
    reasons: [SELF_DEPENDENCY],
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
}

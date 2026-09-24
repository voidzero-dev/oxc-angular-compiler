/**
 * Fixtures whose output differs from Angular's in documented ways, keyed by
 * `category/name`. Each entry lists the static fields that differ and why.
 *
 * A listed fixture is reported as a known difference only while its differences stay inside
 * `fields`, so a new difference elsewhere in the same fixture still fails. A listed fixture
 * that matches Angular fails too, so fixed entries get removed.
 */
interface KnownDifference {
  /** `Class.field` of every static field that differs, e.g. `MyComponent.ɵcmp`. */
  fields: string[]
  /** Why each difference exists. */
  reasons: string[]
}

const FACTORY =
  'ɵfac uses ɵɵgetInheritedFactory where Angular emits a factory that calls the constructor'
const NG_MODULE_IMPORTS = 'ɵmod includes imports, which Angular omits in full compilation'
const HOST_ATTRS_ORDER = 'hostAttrs are emitted in a different order'
const CUSTOM_DECORATOR =
  'the reference ngtsc compile leaves a class with a non-Angular decorator uncompiled, so only Oxc emits ɵfac/ɵcmp'
const CHANGE_DETECTION_V22 =
  'changeDetection is omitted for OnPush, which is the runtime default from v22, where Angular emits it explicitly'
const INCREMENTAL_HYDRATION =
  'ɵɵenableIncrementalHydrationRuntime is not emitted for a `hydrate` trigger'
const SELECTOR_WHITESPACE =
  'runs of whitespace inside shimmed selectors are collapsed where Angular keeps them'
const INJECTABLE_FACTORY_WRAPPER =
  'an @Injectable useFactory is wrapped in a function expression where Angular emits an arrow function'

export const KNOWN_DIFFERENCES: Record<string, KnownDifference> = {
  'animations/animation-metadata-with-change-detection': {
    fields: ['AnimationsWithOnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'class-metadata/class-metadata-ngmodule': {
    fields: ['TestableModule.ɵfac', 'TestableModule.ɵmod'],
    reasons: [FACTORY, NG_MODULE_IMPORTS],
  },
  'class-metadata/class-metadata-pipe': {
    fields: ['TestablePipe.ɵfac'],
    reasons: [FACTORY],
  },
  'component-meta/change-detection-default': {
    fields: ['ChangeDetectionDefaultComponent.ɵcmp'],
    reasons: ['changeDetection is omitted where Angular emits it'],
  },
  'component-meta/change-detection-onpush': {
    fields: ['ChangeDetectionOnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'defer/defer-hydrate-idle': {
    fields: ['DeferHydrateIdleComponent.ɵcmp'],
    reasons: [INCREMENTAL_HYDRATION],
  },
  'defer/defer-hydrate-interaction': {
    fields: ['DeferHydrateInteractionComponent.ɵcmp'],
    reasons: [INCREMENTAL_HYDRATION],
  },
  'defer/defer-hydrate-never': {
    fields: ['DeferHydrateNeverComponent.ɵcmp'],
    reasons: [INCREMENTAL_HYDRATION],
  },
  'defer/defer-hydrate-viewport': {
    fields: ['DeferHydrateViewportComponent.ɵcmp'],
    reasons: [INCREMENTAL_HYDRATION],
  },
  'edge-cases/multiple-custom-decorators': {
    fields: ['MultiDecoratorComponent.ɵcmp', 'MultiDecoratorComponent.ɵfac'],
    reasons: [CUSTOM_DECORATOR],
  },
  'edge-cases/single-custom-decorator': {
    fields: ['MyComponent.ɵcmp', 'MyComponent.ɵfac'],
    reasons: [CUSTOM_DECORATOR],
  },
  'full-file/component-with-pipes': {
    fields: ['ComponentWithPipes.ɵcmp'],
    reasons: ['component styles keep a leading newline'],
  },
  'full-transform/full-transform-complex': {
    fields: ['ComplexComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'full-transform/full-transform-host-attribute': {
    fields: ['HostAttributeComponent.ɵcmp'],
    reasons: [HOST_ATTRS_ORDER],
  },
  'full-transform/full-transform-onpush': {
    fields: ['OnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'full-transform/transform-api-onpush': {
    fields: ['OnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
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
    reasons: [CHANGE_DETECTION_V22],
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
    reasons: [FACTORY, NG_MODULE_IMPORTS],
  },
  'injector/injector-with-imports': {
    fields: ['FeatureModule.ɵfac', 'FeatureModule.ɵmod'],
    reasons: [FACTORY, NG_MODULE_IMPORTS],
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
    reasons: [CHANGE_DETECTION_V22],
  },
  'regressions/class-debug-info-change-detection': {
    fields: ['OnPushDebugInfoComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'regressions/formfield-alias-repro': {
    fields: ['AppComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'schemas/schemas-with-change-detection': {
    fields: ['SchemasWithOnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'styles/shadow-css-host-context-triple': {
    fields: ['HostContextTripleComponent.ɵcmp'],
    reasons: [':host-context selector permutations are in a different order'],
  },
  'styles/shadow-css-host-context-with-descendant': {
    fields: ['HostContextDescendantComponent.ɵcmp'],
    reasons: [SELECTOR_WHITESPACE],
  },
  'styles/shadow-css-multiline-selector': {
    fields: ['MultilineSelectorComponent.ɵcmp'],
    reasons: ['the newline between selectors in a list is dropped'],
  },
  'styles/shadow-css-ng-deep-basic': {
    fields: ['NgDeepBasicComponent.ɵcmp'],
    reasons: ['a leading space is kept before a ::ng-deep selector'],
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
  'styles/shadow-css-where-with-host': {
    fields: ['WhereWithHostComponent.ɵcmp'],
    reasons: [SELECTOR_WHITESPACE],
  },
  'styles/style-onpush-encapsulation': {
    fields: ['OnPushStyleComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'styles/style-onpush-none': {
    fields: ['OnPushNoneStyleComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
}

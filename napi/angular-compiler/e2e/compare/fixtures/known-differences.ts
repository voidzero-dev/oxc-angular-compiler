/**
 * Fixtures whose output differs from Angular's in documented ways, keyed by
 * `category/name`. Each entry lists the static fields that differ and why.
 *
 * A listed fixture is reported as a known difference only while its differences stay inside
 * `fields`, so a new difference elsewhere in the same fixture still fails. A listed fixture
 * that matches Angular fails too, so fixed entries get removed.
 */
import type { ImportDiff } from '../src/compare.js'

interface KnownDifference {
  /** `Class.field` of every static field that differs, e.g. `MyComponent.ɵcmp`. */
  fields: string[]
  /** Every import difference, exactly as `compareImports` reports it. */
  importDiffs?: ImportDiff[]
  /** Every setClassMetadata difference, as `Class.field`, e.g. `MyComponent.setClassMetadata`. */
  metadataDiffs?: string[]
  /** Why each difference exists. */
  reasons: string[]
}

const NG_MODULE_IMPORTS = 'ɵmod includes imports, which Angular omits in full compilation'
const HOST_ATTRS_ORDER = 'hostAttrs are emitted in a different order'
const CUSTOM_DECORATOR =
  'the reference ngtsc compile leaves a class with a non-Angular decorator uncompiled, so only Oxc emits ɵfac/ɵcmp'
const CHANGE_DETECTION_V22 =
  'changeDetection is omitted for OnPush, which is the runtime default from v22, where Angular emits it explicitly'
const SELECTOR_WHITESPACE =
  'runs of whitespace inside shimmed selectors are collapsed where Angular keeps them'
const INJECTABLE_FACTORY_WRAPPER =
  'an @Injectable useFactory is wrapped in a function expression where Angular emits an arrow function'
const SET_CLASS_METADATA_IMPORT =
  'Oxc keeps the @Inject import because its setClassMetadata references it; ngtsc emits the same reference but TypeScript elision still drops the import (upstream emit bug)'
const ALIASED_INJECTABLE =
  'an aliased @Injectable compiles to a broken ɵprov under ngtsc — its needsFactory check compares the written decorator name, so the factory (and setClassMetadata) are never emitted; Oxc emits a working ɵfac (issue #507, kept deliberately)'

export const KNOWN_DIFFERENCES: Record<string, KnownDifference> = {
  'animations/animation-metadata-with-change-detection': {
    fields: ['AnimationsWithOnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
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
  'class-metadata/class-metadata-ngmodule': {
    fields: ['TestableModule.ɵmod'],
    reasons: [NG_MODULE_IMPORTS],
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
    reasons: ['changeDetection is omitted where Angular emits it'],
  },
  'component-meta/change-detection-onpush': {
    fields: ['ChangeDetectionOnPushComponent.ɵcmp'],
    reasons: [CHANGE_DETECTION_V22],
  },
  'edge-cases/aliased-injectable': {
    fields: ['AliasedService.ɵfac'],
    metadataDiffs: ['AliasedService.setClassMetadata'],
    reasons: [ALIASED_INJECTABLE],
  },
  'edge-cases/multiple-custom-decorators': {
    fields: ['MultiDecoratorComponent.ɵcmp', 'MultiDecoratorComponent.ɵfac'],
    metadataDiffs: ['MultiDecoratorComponent.setClassMetadata'],
    reasons: [CUSTOM_DECORATOR],
  },
  'edge-cases/single-custom-decorator': {
    fields: ['MyComponent.ɵcmp', 'MyComponent.ɵfac'],
    metadataDiffs: ['MyComponent.setClassMetadata'],
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
  'injector/injector-with-both': {
    fields: ['FullModule.ɵmod'],
    reasons: [NG_MODULE_IMPORTS],
  },
  'injector/injector-with-imports': {
    fields: ['FeatureModule.ɵmod'],
    reasons: [NG_MODULE_IMPORTS],
  },
  'providers/object-members-injectable-use-factory': {
    fields: ['ObjectMembersService.ɵprov'],
    reasons: [INJECTABLE_FACTORY_WRAPPER],
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

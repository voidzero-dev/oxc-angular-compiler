/**
 * Fixtures for Angular decorators imported under an alias.
 *
 * `import { Injectable as X }` + `@X(...)` is an Angular decorator to Oxc,
 * which resolves the import binding. ngtsc finds the decorator too, but its
 * `needsFactory` check (`injectable.ts`) compares the WRITTEN name
 * (`current.name === 'Injectable'`), so an aliased import compiles to a
 * broken `ɵprov` referencing a `ɵfac` it never emitted — an upstream bug.
 * See https://github.com/voidzero-dev/oxc-angular-compiler/issues/507.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'aliased-injectable',
    category: 'edge-cases',
    description:
      '@Injectable imported under an alias — Oxc emits a working ɵfac and setClassMetadata; ngtsc emits only ɵprov pointing at a factory it never generated (upstream bug)',
    className: 'AliasedService',
    sourceCode: `
import { Injectable as X } from '@angular/core';

@X({ providedIn: 'root' })
export class AliasedService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable'],
  },
]

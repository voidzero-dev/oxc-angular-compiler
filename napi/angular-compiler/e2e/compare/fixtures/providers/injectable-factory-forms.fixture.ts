/**
 * @Injectable factory forms.
 *
 * ngtsc (full compilation) wraps a `useFactory` without `deps` in an expression-body arrow,
 * `() => factory()`, and delegates a `useClass` without `deps` from an arrow,
 * `__ngFactoryType__ => UseClass.ɵfac(__ngFactoryType__)`, whether or not the class is
 * wrapped in `forwardRef()` (it is unwrapped, and the arrow reads it lazily).
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'injectable-use-factory-named-function',
    category: 'providers',
    description: 'Injectable useFactory without deps wraps the factory in an arrow',
    className: 'FactoryService',
    type: 'full-transform',
    sourceCode: `
import { Injectable } from '@angular/core';

export function createFactoryService() {
  return {};
}

@Injectable({ providedIn: 'root', useFactory: createFactoryService })
export class FactoryService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable'],
  },
  {
    name: 'injectable-use-class-other',
    category: 'providers',
    description: 'Injectable useClass without deps delegates to the class factory from an arrow',
    className: 'AbstractLogger',
    type: 'full-transform',
    sourceCode: `
import { Injectable } from '@angular/core';

export class ConsoleLogger {}

@Injectable({ providedIn: 'root', useClass: ConsoleLogger })
export class AbstractLogger {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable'],
  },
  {
    name: 'injectable-use-class-forward-ref',
    category: 'providers',
    description: 'Injectable useClass with forwardRef to a class declared later',
    className: 'EarlyService',
    type: 'full-transform',
    sourceCode: `
import { Injectable, forwardRef } from '@angular/core';

@Injectable({ providedIn: 'root', useClass: forwardRef(() => LateService) })
export class EarlyService {}

export class LateService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable'],
  },
]

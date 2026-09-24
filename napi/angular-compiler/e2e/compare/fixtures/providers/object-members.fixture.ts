/**
 * Object literal members in decorator metadata that are not plain `key: value`
 * pairs: methods, accessors, generators, computed keys and so on.
 *
 * Angular passes these expressions through unchanged, both in the definition
 * and in `setClassMetadata`, so the output must keep every member as written.
 */
import type { Fixture } from '../types.js'

function providerFixture(name: string, description: string, value: string): Fixture {
  return {
    name: `object-members-${name}`,
    category: 'providers',
    description,
    className: 'ObjectMembersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, InjectionToken } from '@angular/core';

const TOKEN = new InjectionToken<unknown>('TOKEN');
const key = 'computedKey';
class Base { base() { return 'base'; } }

@Component({
  selector: 'app-object-members',
  template: '<div></div>',
  providers: [{ provide: TOKEN, useValue: ${value} }],
})
export class ObjectMembersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵProvidersFeature', 'ɵsetClassMetadata'],
  }
}

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Member kinds
  // ==========================================================================

  providerFixture('method', 'Method shorthand', `{ attach() { return 1; } }`),
  providerFixture('getter', 'Getter', `{ get ready() { return true; } }`),
  providerFixture('setter', 'Setter', `{ set ready(value: boolean) {} }`),
  providerFixture(
    'accessor-pair',
    'Getter and setter for the same key',
    `{ _v: 1, get v() { return this._v; }, set v(value: number) { this._v = value; } }`,
  ),
  providerFixture('async-method', 'Async method', `{ async load() { return 1; } }`),
  providerFixture('generator', 'Generator method', `{ *items() { yield 1; } }`),
  providerFixture('async-generator', 'Async generator method', `{ async *stream() { yield 1; } }`),
  providerFixture('computed-method', 'Computed-key method', `{ [key]() { return 1; } }`),
  providerFixture('computed-property', 'Computed-key property', `{ [key]: 1 }`),
  providerFixture(
    'computed-string-literal',
    'Computed key that is a string literal',
    `{ ['literal']: 1 }`,
  ),
  providerFixture(
    'symbol-method',
    'Well-known symbol method',
    `{ *[Symbol.iterator]() { yield 1; } }`,
  ),
  providerFixture(
    'string-and-numeric-method-keys',
    'Methods with string and numeric keys',
    `{ 'with-dash'() { return 1; }, 42() { return 2; } }`,
  ),
  providerFixture(
    'this-and-super',
    'Methods using this and super',
    `Object.setPrototypeOf({ count: 1, read() { return this.count + super.toString().length; } }, new Base())`,
  ),

  // ==========================================================================
  // Signatures and bodies
  // ==========================================================================

  providerFixture(
    'typed-method',
    'Parameter, return and generic type annotations are stripped',
    `{ map<T>(value: T, fallback?: T, ...rest: T[]): T { return value ?? fallback ?? rest[0]; } }`,
  ),
  providerFixture(
    'default-and-destructured-params',
    'Default and destructured parameters',
    `{ configure({ a, b }: { a: number; b: number }, [c]: number[] = [3], d = a + b) { return a + b + c + d; } }`,
  ),
  providerFixture(
    'body-with-template-literal-and-comments',
    'Method body with a template literal and comments',
    `{
      describe(name: string) {
        // explain
        return \`hello \${name}\`; /* trailing */
      },
    }`,
  ),

  // ==========================================================================
  // Mixed and nested objects
  // ==========================================================================

  providerFixture(
    'mixed-members',
    'Spread, shorthand, plain and method members together',
    `{ ...{ spread: 1 }, key, plain: 'x', method() { return key; } }`,
  ),
  providerFixture(
    'nested-in-plain-object',
    'Method object nested inside a plain object and array',
    `{ plain: 1, list: [{ nested() { return 2; } }] }`,
  ),
  providerFixture(
    'call-argument',
    'Method object passed to a call',
    `Object.freeze({ attach() { return 1; } })`,
  ),

  // ==========================================================================
  // Other decorator fields and decorators
  // ==========================================================================

  {
    name: 'object-members-use-factory',
    category: 'providers',
    description: 'Factory returning an object with methods',
    className: 'ObjectMembersFactoryComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, InjectionToken } from '@angular/core';

const TOKEN = new InjectionToken<unknown>('TOKEN');

@Component({
  selector: 'app-object-members-factory',
  template: '<div></div>',
  providers: [{ provide: TOKEN, useFactory: () => ({ attach() { return 1; }, get ready() { return true; } }) }],
})
export class ObjectMembersFactoryComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵProvidersFeature', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-view-providers',
    category: 'providers',
    description: 'viewProviders with methods and computed keys',
    className: 'ObjectMembersViewProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, InjectionToken } from '@angular/core';

const TOKEN = new InjectionToken<unknown>('TOKEN');
const key = 'computedKey';

@Component({
  selector: 'app-object-members-view-providers',
  template: '<div></div>',
  viewProviders: [{ provide: TOKEN, useValue: { attach() { return 1; }, [key]: 2 } }],
})
export class ObjectMembersViewProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵProvidersFeature', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-directive',
    category: 'providers',
    description: 'Directive providers with methods',
    className: 'ObjectMembersDirective',
    type: 'full-transform',
    sourceCode: `
import { Directive, InjectionToken } from '@angular/core';

const TOKEN = new InjectionToken<unknown>('TOKEN');

@Directive({
  selector: '[appObjectMembers]',
  providers: [{ provide: TOKEN, useValue: { attach() { return 1; }, get ready() { return true; } } }],
})
export class ObjectMembersDirective {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineDirective', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-ng-module',
    category: 'providers',
    description: 'NgModule providers with methods',
    className: 'ObjectMembersModule',
    type: 'full-transform',
    sourceCode: `
import { NgModule, InjectionToken } from '@angular/core';

const TOKEN = new InjectionToken<unknown>('TOKEN');
const key = 'computedKey';

@NgModule({
  providers: [{ provide: TOKEN, useValue: { async load() { return 1; }, [key]() { return 2; } } }],
})
export class ObjectMembersModule {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjector', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-injectable-use-factory',
    category: 'providers',
    description: 'Injectable useFactory returning an object with methods',
    className: 'ObjectMembersService',
    type: 'full-transform',
    sourceCode: `
import { Injectable } from '@angular/core';

@Injectable({
  providedIn: 'root',
  useFactory: () => ({ attach() { return 1; }, get ready() { return true; } }),
})
export class ObjectMembersService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-injectable-use-value',
    category: 'providers',
    description: 'Injectable useValue with methods',
    className: 'ObjectMembersValueService',
    type: 'full-transform',
    sourceCode: `
import { Injectable } from '@angular/core';

@Injectable({
  providedIn: 'root',
  useValue: { attach() { return 1; } },
})
export class ObjectMembersValueService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-host-computed-key',
    category: 'providers',
    description: 'host metadata with a computed string key',
    className: 'ObjectMembersHostComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-object-members-host',
  template: '<div></div>',
  host: { ['(click)']: 'onClick()', class: 'plain' },
})
export class ObjectMembersHostComponent {
  onClick() {}
}
    `.trim(),
    expectedFeatures: ['hostBindings', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-animations',
    category: 'providers',
    description: 'animations metadata containing an object with a method',
    className: 'ObjectMembersAnimationsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-object-members-animations',
  template: '<div></div>',
  animations: [{ type: 7, name: 'fade', definitions: [], options: { params: { get delay() { return 1; } } } }],
})
export class ObjectMembersAnimationsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'ɵsetClassMetadata'],
  },

  // ==========================================================================
  // Methods and accessors on a decorator's own options object are ignored,
  // like ngtsc's reflectObjectLiteral. setClassMetadata keeps them.
  // (`@Input({ transform() {} })` is a compile error in ngc, NG1010.)
  // ==========================================================================

  {
    name: 'object-members-options-injectable',
    category: 'providers',
    description: '@Injectable options with a useFactory method and a useValue getter',
    className: 'OptionsMethodService',
    type: 'full-transform',
    sourceCode: `
import { Injectable } from '@angular/core';

@Injectable({ providedIn: 'root', useFactory() { return 1; } })
export class OptionsMethodService {}

@Injectable({ providedIn: 'root', get useValue() { return 1; } })
export class OptionsGetterService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-options-directive-pipe-module',
    category: 'providers',
    description: 'Accessors on @Directive, @Pipe and @NgModule options',
    className: 'OptionsDirective',
    type: 'full-transform',
    sourceCode: `
import { Directive, NgModule, Pipe } from '@angular/core';

@Directive({ selector: '[appOptions]', get providers() { return []; } })
export class OptionsDirective {}

@Pipe({ name: 'options', get pure() { return false; } })
export class OptionsPipe {
  transform(value: unknown) { return value; }
}

@NgModule({ get providers() { return []; } })
export class OptionsModule {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineDirective', 'ɵɵdefinePipe', 'ɵsetClassMetadata'],
  },

  {
    name: 'object-members-options-component-queries',
    category: 'providers',
    description: 'Accessors on @Component and query options',
    className: 'OptionsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ContentChild, InjectionToken, ViewChild } from '@angular/core';

const TOKEN = new InjectionToken<unknown>('TOKEN');

@Component({
  selector: 'app-options',
  template: '<div #x></div>',
  get viewProviders() { return []; },
})
export class OptionsComponent {
  @ViewChild('x', { static: true, get read() { return TOKEN; } }) x: unknown;
  @ContentChild('y', { get descendants() { return false; } }) y: unknown;
}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'viewQuery', 'ɵsetClassMetadata'],
  },
]

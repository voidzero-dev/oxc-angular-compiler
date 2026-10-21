/**
 * Provider configuration fixtures.
 *
 * Tests different provider configurations in @Component decorator:
 * - providers: Component-level dependency injection providers
 * - viewProviders: View-only providers (available to view children, not content children)
 *
 * Provider configurations appear in the component definition's providers/viewProviders arrays.
 * These affect how Angular's DI system resolves dependencies for the component and its children.
 *
 * NOTE: The actual provider expressions are passed through to the compiled output.
 * These fixtures test that the compiler correctly handles the providers metadata,
 * not the actual DI runtime behavior.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic Providers Array
  // ==========================================================================

  {
    name: 'providers-basic-class',
    category: 'providers',
    description: 'Component with basic class provider',
    className: 'BasicProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class DataService {
  value = 'data';
}

@Component({
  selector: 'app-providers-basic-class',
  standalone: true,
  template: \`<div>{{ service.value }}</div>\`,
  providers: [DataService],
})
export class BasicProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-multiple',
    category: 'providers',
    description: 'Component with multiple providers',
    className: 'MultipleProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class ServiceA {
  value = 'A';
}
class ServiceB {
  value = 'B';
}
class ServiceC {
  value = 'C';
}

@Component({
  selector: 'app-providers-multiple',
  standalone: true,
  template: \`<div>{{ serviceA.value }} - {{ serviceB.value }}</div>\`,
  providers: [ServiceA, ServiceB, ServiceC],
})
export class MultipleProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-use-class',
    category: 'providers',
    description: 'Provider with useClass configuration',
    className: 'UseClassProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class Logger {
  log(msg: string) { return msg; }
}
class ConsoleLogger extends Logger {
  override log(msg: string) { console.log(msg); return msg; }
}

@Component({
  selector: 'app-providers-use-class',
  standalone: true,
  template: \`<div>{{ logger.log('test') }}</div>\`,
  providers: [{ provide: Logger, useClass: ConsoleLogger }],
})
export class UseClassProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-use-value',
    category: 'providers',
    description: 'Provider with useValue configuration',
    className: 'UseValueProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, InjectionToken } from '@angular/core';

interface AppConfig {
  apiUrl: string;
}

const APP_CONFIG = new InjectionToken<AppConfig>('APP_CONFIG');

@Component({
  selector: 'app-providers-use-value',
  standalone: true,
  template: \`<div>{{ config.apiUrl }}</div>\`,
  providers: [{ provide: APP_CONFIG, useValue: { apiUrl: 'https://api.example.com' } }],
})
export class UseValueProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-use-factory',
    category: 'providers',
    description: 'Provider with useFactory configuration',
    className: 'UseFactoryProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class HttpClient {}
class DataService {
  data = 'factory data';
}

function dataServiceFactory(http: HttpClient) {
  return new DataService();
}

@Component({
  selector: 'app-providers-use-factory',
  standalone: true,
  template: \`<div>{{ service.data }}</div>\`,
  providers: [{ provide: DataService, useFactory: dataServiceFactory, deps: [HttpClient] }],
})
export class UseFactoryProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-use-existing',
    category: 'providers',
    description: 'Provider with useExisting configuration',
    className: 'UseExistingProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class AbstractFormatter {
  transform(value: string) { return value; }
}
class ConcreteFormatter extends AbstractFormatter {
  override transform(value: string) { return value.toUpperCase(); }
}

@Component({
  selector: 'app-providers-use-existing',
  standalone: true,
  template: \`<div>{{ service.transform('test') }}</div>\`,
  providers: [{ provide: AbstractFormatter, useExisting: ConcreteFormatter }],
})
export class UseExistingProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-multi',
    category: 'providers',
    description: 'Multi-provider configuration',
    className: 'MultiProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, InjectionToken } from '@angular/core';

interface Validator {
  name: string;
}

const VALIDATORS = new InjectionToken<Validator[]>('VALIDATORS');

class RequiredValidator implements Validator {
  name = 'required';
}
class MinLengthValidator implements Validator {
  name = 'minLength';
}

@Component({
  selector: 'app-providers-multi',
  standalone: true,
  template: \`
    @for (validator of validators; track $index) {
      <div>{{ validator.name }}</div>
    }
  \`,
  providers: [
    { provide: VALIDATORS, useClass: RequiredValidator, multi: true },
    { provide: VALIDATORS, useClass: MinLengthValidator, multi: true },
  ],
})
export class MultiProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:', 'ɵɵrepeaterCreate'],
  },

  // ==========================================================================
  // View Providers
  // ==========================================================================

  {
    name: 'view-providers-basic',
    category: 'providers',
    description: 'Component with basic viewProviders',
    className: 'ViewProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class ViewService {
  value = 'view service';
}

@Component({
  selector: 'app-view-providers-basic',
  standalone: true,
  template: \`<div>{{ viewService.value }}</div>\`,
  viewProviders: [ViewService],
})
export class ViewProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'viewProviders:'],
  },

  {
    name: 'view-providers-multiple',
    category: 'providers',
    description: 'Component with multiple viewProviders',
    className: 'MultipleViewProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class ViewServiceA {
  value = 'A';
}
class ViewServiceB {
  value = 'B';
}

@Component({
  selector: 'app-view-providers-multiple',
  standalone: true,
  template: \`<div>{{ serviceA.value }} - {{ serviceB.value }}</div>\`,
  viewProviders: [ViewServiceA, ViewServiceB],
})
export class MultipleViewProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'viewProviders:'],
  },

  {
    name: 'view-providers-with-use-class',
    category: 'providers',
    description: 'ViewProvider with useClass configuration',
    className: 'ViewProviderUseClassComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class Formatter {
  format(value: string) { return value; }
}
class ViewFormatter extends Formatter {
  override format(value: string) { return \`[VIEW] \${value}\`; }
}

@Component({
  selector: 'app-view-providers-with-use-class',
  standalone: true,
  template: \`<div>{{ formatter.format(value) }}</div>\`,
  viewProviders: [{ provide: Formatter, useClass: ViewFormatter }],
})
export class ViewProviderUseClassComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'viewProviders:'],
  },

  // ==========================================================================
  // Combined Providers and ViewProviders
  // ==========================================================================

  {
    name: 'providers-and-view-providers',
    category: 'providers',
    description: 'Component with both providers and viewProviders',
    className: 'CombinedProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class SharedService {
  value = 'shared';
}
class ViewOnlyService {
  value = 'view-only';
}

@Component({
  selector: 'app-providers-and-view-providers',
  standalone: true,
  template: \`
    <div>Shared: {{ sharedService.value }}</div>
    <div>View-only: {{ viewOnlyService.value }}</div>
  \`,
  providers: [SharedService],
  viewProviders: [ViewOnlyService],
})
export class CombinedProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:', 'viewProviders:'],
  },

  {
    name: 'providers-complex-combined',
    category: 'providers',
    description: 'Complex provider configuration with both types',
    className: 'ComplexCombinedProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class DataService {
  data = 'data';
}
class Logger {
  log(msg: string) { console.log(msg); }
}
class ConsoleLogger extends Logger {}
class ViewFormatter {
  format(value: string) { return value; }
}

function formatterFactory() {
  return new ViewFormatter();
}

@Component({
  selector: 'app-providers-complex-combined',
  standalone: true,
  template: \`
    <div>{{ dataService.data }}</div>
    <ng-content></ng-content>
  \`,
  providers: [DataService, { provide: Logger, useClass: ConsoleLogger }],
  viewProviders: [{ provide: ViewFormatter, useFactory: formatterFactory }],
})
export class ComplexCombinedProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:', 'viewProviders:', 'ɵɵprojection'],
  },

  // ==========================================================================
  // Providers with Other Component Metadata
  // ==========================================================================

  {
    name: 'providers-with-encapsulation',
    category: 'providers',
    description: 'Providers with ViewEncapsulation.None',
    className: 'ProvidersWithEncapsulationComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

class GlobalService {
  value = 'global';
}

@Component({
  selector: 'app-providers-with-encapsulation',
  standalone: true,
  template: \`<div class="global">{{ service.value }}</div>\`,
  providers: [GlobalService],
  encapsulation: ViewEncapsulation.None,
})
export class ProvidersWithEncapsulationComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:', 'encapsulation: 2'],
  },

  {
    name: 'providers-with-change-detection',
    ngtscMode: 'full',
    category: 'providers',
    description: 'Providers with OnPush change detection',
    className: 'ProvidersWithChangeDetectionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';

class AsyncService {
  value$ = 'async value';
}

@Component({
  selector: 'app-providers-with-change-detection',
  standalone: true,
  template: \`<div>{{ service.value$ | async }}</div>\`,
  providers: [AsyncService],
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ProvidersWithChangeDetectionComponent {}
    `.trim(),
    expectedFeatures: [
      'ɵɵdefineComponent',
      'providers:',
      'ChangeDetectionStrategy.OnPush',
      'ɵɵpipe',
    ],
  },

  {
    name: 'providers-with-host-bindings',
    category: 'providers',
    description: 'Providers with host bindings',
    className: 'ProvidersWithHostBindingsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

class StyleService {
  getStyle() { return 'styled'; }
}

@Component({
  selector: 'app-providers-with-host-bindings',
  standalone: true,
  template: \`<div>{{ value }}</div>\`,
  providers: [StyleService],
  host: {
    '[class.styled]': 'isStyled',
    '[style.color]': 'color',
  },
})
export class ProvidersWithHostBindingsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  // ==========================================================================
  // Edge Cases
  // ==========================================================================

  {
    name: 'providers-empty-array',
    category: 'providers',
    description: 'Component with empty providers array',
    className: 'EmptyProvidersComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-providers-empty-array',
  standalone: true,
  template: \`<div>No providers</div>\`,
  providers: [],
})
export class EmptyProvidersComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },

  {
    name: 'providers-injection-token',
    category: 'providers',
    description: 'Provider using InjectionToken',
    className: 'InjectionTokenProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, InjectionToken } from '@angular/core';

interface ApiConfig {
  apiUrl: string;
}

const API_CONFIG_TOKEN = new InjectionToken<ApiConfig>('API_CONFIG_TOKEN');

@Component({
  selector: 'app-providers-injection-token',
  standalone: true,
  template: \`<div>{{ config.apiUrl }}</div>\`,
  providers: [{ provide: API_CONFIG_TOKEN, useValue: { apiUrl: '/api' } }],
})
export class InjectionTokenProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'providers-forward-ref',
    category: 'providers',
    description: 'Provider using forwardRef for circular dependencies',
    className: 'ForwardRefProviderComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, forwardRef } from '@angular/core';

class ParentService {
  value = 'parent';
}
class ChildService extends ParentService {
  override value = 'child';
}

@Component({
  selector: 'app-providers-forward-ref',
  standalone: true,
  template: \`<div>{{ service.value }}</div>\`,
  providers: [{ provide: ParentService, useExisting: forwardRef(() => ChildService) }],
})
export class ForwardRefProviderComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent', 'providers:'],
  },

  {
    name: 'injectable-factory-without-constructor',
    category: 'providers',
    description:
      'Injectable without a constructor: a plain factory, or the inherited one when it extends a class',
    className: 'PlainService',
    type: 'full-transform',
    sourceCode: `
import { Injectable } from '@angular/core';

class BaseService {}

@Injectable({ providedIn: 'root' })
export class PlainService {}

@Injectable({ providedIn: 'root' })
export class DerivedService extends BaseService {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineInjectable', 'ɵɵgetInheritedFactory'],
  },
]

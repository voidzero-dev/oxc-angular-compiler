/**
 * Custom (non-Angular) class decorators.
 *
 * Tests that non-Angular decorators are preserved in the Angular compiler output
 * without breaking the generated code. The Angular compiler strips @Component
 * but must leave custom decorators intact for downstream TS-to-JS tools
 * (e.g., Rolldown) to lower.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'single-custom-decorator',
    category: 'edge-cases',
    description: 'Component with a single custom class decorator',
    className: 'MyComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

function Log(message: string) {
  return function <T extends new (...args: any[]) => any>(target: T): T {
    console.log(message);
    return target;
  };
}

@Log('MyComponent loaded')
@Component({
  selector: 'app-my',
  template: '<span>hello</span>',
})
export class MyComponent {}
`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵfac'],
  },
  {
    name: 'multiple-custom-decorators',
    category: 'edge-cases',
    description: 'Component with multiple custom class decorators',
    className: 'MultiDecoratorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

function Sealed(target: any) { Object.seal(target); return target; }
function Track(name: string) {
  return function(target: any) { return target; };
}

@Sealed
@Track('multi')
@Component({
  selector: 'app-multi',
  template: '<div>multi</div>',
})
export class MultiDecoratorComponent {}
`,
    expectedFeatures: ['ɵɵdefineComponent', 'ɵfac'],
  },
]

/**
 * Custom (non-Angular) class decorators.
 *
 * Tests that non-Angular decorators are properly lowered to JavaScript
 * when the `typescript` transform option is enabled.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'single-custom-decorator',
    category: 'edge-cases',
    description: 'Component with a single custom class decorator',
    className: 'CustomDecoratorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

function TrackChanges() {
  return function(target: any) { return target; };
}

@TrackChanges()
@Component({
  selector: 'app-custom-decorator',
  standalone: true,
  template: \`<span>Custom decorator test</span>\`,
})
export class CustomDecoratorComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },
  {
    name: 'multiple-custom-decorators',
    category: 'edge-cases',
    description: 'Component with multiple custom class decorators',
    className: 'MultiDecoratorComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

function Log(message: string) {
  return function(target: any) { return target; };
}

function Sealed() {
  return function(target: any) {
    Object.seal(target);
    Object.seal(target.prototype);
    return target;
  };
}

@Log('Multi decorator component')
@Sealed()
@Component({
  selector: 'app-multi-decorator',
  standalone: true,
  template: \`<span>Multiple decorators</span>\`,
})
export class MultiDecoratorComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },
]

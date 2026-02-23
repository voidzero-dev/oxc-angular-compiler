/**
 * @defer trigger edge cases for compiler divergence testing.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'defer-viewport-trigger-only',
    category: 'defer',
    description: '@defer viewport with trigger option only (should keep empty options object)',
    className: 'DeferViewportTriggerOnlyComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-viewport-trigger-only',
  standalone: true,
  template: \`
    <div #myRef>Trigger element</div>
    @defer (on viewport({trigger: myRef})) {
      <div>Deferred</div>
    }
  \`,
})
export class DeferViewportTriggerOnlyComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnViewport'],
  },
  {
    name: 'defer-timer-fractional-ms',
    category: 'defer',
    description: '@defer timer with fractional milliseconds (1.5ms)',
    className: 'DeferTimerFractionalMsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-timer-fractional-ms',
  standalone: true,
  template: \`
    @defer (on timer(1.5ms)) {
      <div>Timer triggered</div>
    }
  \`,
})
export class DeferTimerFractionalMsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnTimer'],
  },
  {
    name: 'defer-timer-fractional-s',
    category: 'defer',
    description: '@defer timer with fractional seconds (1.5s = 1500ms)',
    className: 'DeferTimerFractionalSComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-timer-fractional-s',
  standalone: true,
  template: \`
    @defer (on timer(1.5s)) {
      <div>Timer triggered</div>
    }
  \`,
})
export class DeferTimerFractionalSComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnTimer'],
  },
]

/**
 * @defer blocks with various trigger types.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'defer-on-viewport',
    category: 'defer',
    description: '@defer with on viewport trigger',
    className: 'DeferViewportComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-viewport',
  standalone: true,
  template: \`
    @defer (on viewport) {
      <div>Visible in viewport</div>
    } @placeholder {
      <span>Loading...</span>
    }
  \`,
})
export class DeferViewportComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnViewport'],
  },
  {
    name: 'defer-on-hover',
    category: 'defer',
    description: '@defer with on hover trigger',
    className: 'DeferHoverComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-hover',
  standalone: true,
  template: \`
    <button #trigger>Hover me</button>
    @defer (on hover(trigger)) {
      <div>Hovered content</div>
    }
  \`,
})
export class DeferHoverComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnHover'],
  },
  {
    name: 'defer-on-interaction',
    category: 'defer',
    description: '@defer with on interaction trigger',
    className: 'DeferInteractionComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-interaction',
  standalone: true,
  template: \`
    <button #btn>Click me</button>
    @defer (on interaction(btn)) {
      <div>Interaction triggered</div>
    }
  \`,
})
export class DeferInteractionComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnInteraction'],
  },
  {
    name: 'defer-on-timer',
    category: 'defer',
    description: '@defer with on timer trigger',
    className: 'DeferTimerComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-timer',
  standalone: true,
  template: \`
    @defer (on timer(500ms)) {
      <div>Timer triggered</div>
    }
  \`,
})
export class DeferTimerComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnTimer'],
  },
  {
    name: 'defer-on-immediate',
    category: 'defer',
    description: '@defer with on immediate trigger',
    className: 'DeferImmediateComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-immediate',
  standalone: true,
  template: \`
    @defer (on immediate) {
      <div>Immediately loaded</div>
    }
  \`,
})
export class DeferImmediateComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnImmediate'],
  },
  {
    name: 'defer-on-idle-timeout',
    category: 'defer',
    description: '@defer with idle timeouts on the main, prefetch and hydrate triggers',
    className: 'DeferIdleTimeoutComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-idle-timeout',
  standalone: true,
  template: \`
    @defer (on idle(100ms); prefetch on idle(2s)) {
      <div>Loaded when idle</div>
    }
    @defer (on idle; hydrate on idle(250)) {
      <div>Hydrated when idle</div>
    }
  \`,
})
export class DeferIdleTimeoutComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnIdle', 'ɵɵdeferPrefetchOnIdle', 'ɵɵdeferHydrateOnIdle'],
  },
]

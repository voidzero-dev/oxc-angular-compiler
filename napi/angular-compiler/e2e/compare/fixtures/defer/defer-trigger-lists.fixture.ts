/**
 * @defer `on` trigger lists: several comma-separated triggers per parameter,
 * whitespace and newlines between them, and commas nested inside trigger
 * arguments (strings, arrays, object literals).
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'defer-on-trigger-list',
    category: 'defer',
    description: '@defer with several comma-separated `on` triggers across lines',
    className: 'DeferOnTriggerListComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-on-trigger-list',
  standalone: true,
  template: \`
    <button #btn>Load</button>
    @defer (on  idle ,  timer(500ms) ;
            prefetch on
              hover(btn),
              interaction(btn)) {
      <div>Deferred</div>
    } @placeholder {
      <span>Placeholder</span>
    }
  \`,
})
export class DeferOnTriggerListComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnIdle', 'ɵɵdeferOnTimer', 'ɵɵdeferPrefetchOnHover'],
  },
  {
    name: 'defer-viewport-options-nested-commas',
    category: 'defer',
    description: '@defer viewport options whose strings and arrays contain commas',
    className: 'DeferViewportOptionsNestedCommasComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-viewport-options-nested-commas',
  standalone: true,
  template: \`
    <div #target>Target</div>
    @defer (on idle, viewport({trigger: target, rootMargin: '1px, 2px', threshold: [0, 0.5, 1]})) {
      <div>Deferred</div>
    }
  \`,
})
export class DeferViewportOptionsNestedCommasComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferOnIdle', 'ɵɵdeferOnViewport'],
  },
  {
    name: 'defer-prefetch-hydrate-trigger-lists',
    category: 'defer',
    description: '@defer with `prefetch on` and `hydrate on` trigger lists',
    className: 'DeferPrefetchHydrateTriggerListsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-defer-prefetch-hydrate-trigger-lists',
  standalone: true,
  template: \`
    <div #anchor>Anchor</div>
    @defer (on timer(1.5s), hover(anchor);
            prefetch on hover(anchor), viewport({rootMargin: '10px'});
            hydrate on idle, timer(1s), immediate) {
      <div>Deferred</div>
    } @placeholder {
      <span>Placeholder</span>
    }
  \`,
})
export class DeferPrefetchHydrateTriggerListsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefer', 'ɵɵdeferPrefetchOnViewport', 'ɵɵdeferHydrateOnTimer'],
  },
]

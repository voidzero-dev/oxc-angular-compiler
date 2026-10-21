/**
 * Change detection strategy fixtures.
 *
 * Tests different change detection strategies:
 * - Default: Check on every change detection cycle
 * - OnPush: Check only on input changes or events
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'change-detection-default',
    ngtscMode: 'full',
    category: 'component-meta',
    description: 'ChangeDetectionStrategy.Default - check on every cycle',
    className: 'ChangeDetectionDefaultComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-change-detection-default',
  standalone: true,
  template: \`<div class="container">{{message}}</div>\`,
  changeDetection: ChangeDetectionStrategy.Default,
})
export class ChangeDetectionDefaultComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },
  {
    name: 'change-detection-onpush',
    ngtscMode: 'full',
    category: 'component-meta',
    description: 'ChangeDetectionStrategy.OnPush - check only on input changes',
    className: 'ChangeDetectionOnPushComponent',
    type: 'full-transform',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-change-detection-onpush',
  standalone: true,
  template: \`<div class="container">{{message}}</div>\`,
  changeDetection: ChangeDetectionStrategy.OnPush,
})
export class ChangeDetectionOnPushComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵdefineComponent'],
  },
]

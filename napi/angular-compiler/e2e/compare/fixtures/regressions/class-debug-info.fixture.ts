/**
 * Regression: setClassDebugInfo emission.
 *
 * Angular generates dev-mode metadata calls for components:
 * - setClassMetadata: Stores decorator info for reflection
 * - setClassDebugInfo: Stores className, filePath, lineNumber for error messages
 *
 * Expected output includes:
 * ```javascript
 * (() => {
 *   (typeof ngDevMode === "undefined" || ngDevMode) &&
 *     i0.ɵsetClassDebugInfo(ComponentName, {
 *       className: "ComponentName",
 *       filePath: "path/to/file.ts",
 *       lineNumber: N
 *     });
 * })();
 * ```
 *
 * Found in: Bitwarden/ClickUp comparisons
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    type: 'full-transform',
    name: 'class-debug-info-basic',
    category: 'regressions',
    description: 'Basic component should emit setClassDebugInfo',
    className: 'BasicDebugInfoComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-basic-debug',
  standalone: true,
  template: \`<div>Hello</div>\`,
})
export class BasicDebugInfoComponent {}
`.trim(),
    expectedFeatures: ['ɵsetClassDebugInfo', 'ɵsetClassMetadata'],
  },
  {
    type: 'full-transform',
    name: 'class-debug-info-with-inputs',
    category: 'regressions',
    description: 'Component with inputs should emit setClassDebugInfo',
    className: 'InputDebugInfoComponent',
    sourceCode: `
import { Component, Input } from '@angular/core';

@Component({
  selector: 'app-input-debug',
  standalone: true,
  template: \`<div>{{ name }}</div>\`,
})
export class InputDebugInfoComponent {
  @Input() name: string = '';
}
`.trim(),
    expectedFeatures: ['ɵsetClassDebugInfo', 'ɵsetClassMetadata'],
  },
  {
    type: 'full-transform',
    name: 'class-debug-info-change-detection',
    ngtscMode: 'full',
    category: 'regressions',
    description: 'Component with OnPush should emit setClassDebugInfo',
    className: 'OnPushDebugInfoComponent',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-onpush-debug',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: \`<div>OnPush</div>\`,
})
export class OnPushDebugInfoComponent {}
`.trim(),
    expectedFeatures: ['ɵsetClassDebugInfo', 'ɵsetClassMetadata'],
  },
]

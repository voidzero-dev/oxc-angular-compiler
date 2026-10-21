/**
 * Full Transform API fixtures.
 *
 * These fixtures use type: "full-transform" to test the transformAngularFile() NAPI
 * which performs complete TypeScript file transformation including:
 * - Decorator extraction from class definitions
 * - Template compilation integrated with class metadata
 * - Full JavaScript output generation
 *
 * This is different from template-only compilation which just compiles the template string.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  // ==========================================================================
  // Basic transformAngularFile Tests
  // ==========================================================================

  {
    type: 'full-transform',
    name: 'transform-api-basic',
    category: 'full-transform',
    description: 'Basic full file transformation using transformAngularFile API',
    className: 'BasicTransformComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-basic-transform',
  standalone: true,
  template: \`<div>{{ message }}</div>\`,
})
export class BasicTransformComponent {
  message = 'Hello World';
}
    `.trim(),
    // Template-level features (we extract just the template function for comparison)
    expectedFeatures: ['ɵɵelementStart', 'ɵɵtext', 'ɵɵtextInterpolate'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-with-host',
    category: 'full-transform',
    description: 'Full file transformation with host bindings in decorator',
    className: 'HostBindingComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-host-binding',
  standalone: true,
  host: {
    '[class.active]': 'isActive',
    '(click)': 'handleClick()',
  },
  template: \`<div>Host bindings test</div>\`,
})
export class HostBindingComponent {
  isActive = true;
  handleClick() {}
}
    `.trim(),
    // Template-level features (host bindings are in hostBindings function, not template)
    expectedFeatures: ['ɵɵelement'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-with-control-flow',
    category: 'full-transform',
    description: 'Full file transformation with @if control flow',
    className: 'ControlFlowComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-control-flow',
  standalone: true,
  template: \`
    @if (showContent) {
      <div>Content is visible</div>
    } @else {
      <div>Content is hidden</div>
    }
  \`,
})
export class ControlFlowComponent {
  showContent = true;
}
    `.trim(),
    // Template-level features
    expectedFeatures: ['ɵɵconditional'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-with-for',
    category: 'full-transform',
    description: 'Full file transformation with @for loop',
    className: 'ForLoopComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-for-loop',
  standalone: true,
  template: \`
    @for (item of items; track item.id) {
      <div>{{ item.name }}</div>
    }
  \`,
})
export class ForLoopComponent {
  items = [{ id: 1, name: 'Item 1' }, { id: 2, name: 'Item 2' }];
}
    `.trim(),
    // Template-level features
    expectedFeatures: ['ɵɵrepeaterCreate'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-encapsulation-none',
    category: 'full-transform',
    description: 'Full file transformation with ViewEncapsulation.None',
    className: 'NoEncapsulationComponent',
    sourceCode: `
import { Component, ViewEncapsulation } from '@angular/core';

@Component({
  selector: 'app-no-encapsulation',
  standalone: true,
  encapsulation: ViewEncapsulation.None,
  template: \`<div class="global">Global styles</div>\`,
})
export class NoEncapsulationComponent {}
    `.trim(),
    // Template-level features (encapsulation is a component def property, not template)
    expectedFeatures: ['ɵɵelement'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-onpush',
    ngtscMode: 'full',
    category: 'full-transform',
    description: 'Full file transformation with OnPush change detection',
    className: 'OnPushComponent',
    sourceCode: `
import { Component, ChangeDetectionStrategy } from '@angular/core';

@Component({
  selector: 'app-onpush',
  standalone: true,
  changeDetection: ChangeDetectionStrategy.OnPush,
  template: \`<div>{{ data }}</div>\`,
})
export class OnPushComponent {
  data = 'OnPush data';
}
    `.trim(),
    // Template-level features (changeDetection is a component def property, not template)
    expectedFeatures: ['ɵɵelementStart', 'ɵɵtext', 'ɵɵtextInterpolate'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-with-styles',
    category: 'full-transform',
    description: 'Full file transformation with inline styles',
    className: 'StyledComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-styled',
  standalone: true,
  styles: [\`.container { padding: 1rem; }\`],
  template: \`<div class="container">Styled content</div>\`,
})
export class StyledComponent {}
    `.trim(),
    // Template-level features (styles is a component def property, not template)
    expectedFeatures: ['ɵɵelement'],
  },

  {
    type: 'full-transform',
    name: 'transform-api-factory',
    category: 'full-transform',
    description: 'Full file transformation generates factory function',
    className: 'FactoryComponent',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-factory',
  standalone: true,
  template: \`<div>Factory test</div>\`,
})
export class FactoryComponent {}
    `.trim(),
    // Template-level features (factory is generated separately)
    expectedFeatures: ['ɵɵelement'],
  },
]

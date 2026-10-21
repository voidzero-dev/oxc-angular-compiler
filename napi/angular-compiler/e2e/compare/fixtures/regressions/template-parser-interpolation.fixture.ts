/**
 * Regression: template parser interpolation detection.
 *
 * Issue: a pre-scan required `{{` before the first `}}`, so a value such as
 * `}} {{a}}` stayed static where Angular's BindingParser binds it.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'interpolation-after-closing-braces-attribute',
    category: 'regressions',
    description: 'Attribute with `}}` before its interpolation',
    className: 'InterpolationAfterClosingBracesAttributeComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-interpolation-after-closing-braces-attribute',
  standalone: true,
  template: \`
      <div title="}} {{a}}"></div>
      <span attr.aria-label="}} {{a}} {{b}}"></span>
    \`,
})
export class InterpolationAfterClosingBracesAttributeComponent {
  a = 'a';
  b = 'b';
}
    `.trim(),
    expectedFeatures: ['ɵɵinterpolate1'],
  },
  {
    name: 'interpolation-after-closing-braces-text',
    category: 'regressions',
    description: 'Text with encoded `}}` before its interpolation (a bare `}` closes a block)',
    className: 'InterpolationAfterClosingBracesTextComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-interpolation-after-closing-braces-text',
  standalone: true,
  template: \`
      <p>&#125;&#125; {{a}}</p>
      <p>&#125; {{a}} &#125;&#125;</p>
    \`,
})
export class InterpolationAfterClosingBracesTextComponent {
  a = 'a';
}
    `.trim(),
    expectedFeatures: ['ɵɵtextInterpolate1'],
  },
  {
    name: 'interpolation-after-closing-braces-i18n',
    category: 'regressions',
    description: 'i18n text and attribute with `}}` before the interpolation',
    className: 'InterpolationAfterClosingBracesI18nComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-interpolation-after-closing-braces-i18n',
  standalone: true,
  template: \`
      <p i18n>&#125;&#125; {{a}}</p>
      <div i18n-title title="}} {{a}}"></div>
    \`,
})
export class InterpolationAfterClosingBracesI18nComponent {
  a = 'a';
}
    `.trim(),
    expectedFeatures: ['ɵɵi18n', 'ɵɵi18nAttributes'],
  },
]

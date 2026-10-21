/**
 * $localize raw text.
 *
 * ngtsc emits `$localize` as a native tagged template for ES2015+ targets and prints each
 * part's raw text as Angular's `createCookedRawString` escapes it: backslashes doubled,
 * backticks and `${` escaped, colons escaped in the metadata block (and a leading colon of a
 * part without one), and line terminators kept as they are.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'i18n-localize-raw-text-escapes',
    category: 'i18n',
    description: 'Backticks, ${, backslashes and colons in $localize raw text',
    className: 'LocalizeRawTextComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-localize-raw-text',
  standalone: true,
  template: '<div i18n="mean:ing|desc:ription@@raw-id">a \` b $&#123; c &#125; d \\\\ e : {{ name }} f \\\\\` g</div><span i18n>:starts with a colon</span><p i18n-title title="x \` y \\\\ z">t</p>',
})
export class LocalizeRawTextComponent {
  name = 'n';
}
    `.trim(),
    expectedFeatures: ['ɵɵi18n', '$localize'],
  },
  {
    name: 'i18n-localize-raw-text-line-terminators',
    category: 'i18n',
    description: 'Line feeds and LS/PS stay as they are in $localize raw text',
    className: 'LocalizeLineTerminatorsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-localize-line-terminators',
  standalone: true,
  preserveWhitespaces: true,
  template: '<div i18n>one\\ntwo\\u2028three\\u2029four</div>',
})
export class LocalizeLineTerminatorsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵi18n', '$localize'],
  },
]

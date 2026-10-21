/**
 * Regression: `i18n` metadata trimming.
 *
 * Issue: the `i18n` metadata parser trimmed the meaning and description
 * separately; Angular's `parseI18nMeta` trims the whole value once, so the
 * parts keep their inner spaces in `$localize` and JSDoc.
 */
import type { Fixture } from '../types.js'

export const fixtures: Fixture[] = [
  {
    name: 'i18n-meta-untrimmed-parts',
    category: 'regressions',
    description: 'i18n meaning and description keep inner spaces after the value is trimmed',
    className: 'I18nMetaUntrimmedPartsComponent',
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-i18n-meta-untrimmed-parts',
  standalone: true,
  template: \`
      <p i18n="  m | d  ">Hello</p>
      <p i18n="  only description  ">World</p>
      <p i18n=" m | d @@ custom ">Custom</p>
      <p i18n="d@@id|x">Pipe after id</p>
      <img i18n-alt="  m | d  " alt="Logo" />
    \`,
})
export class I18nMetaUntrimmedPartsComponent {}
    `.trim(),
    expectedFeatures: ['ɵɵi18n', 'ɵɵi18nAttributes'],
  },
]

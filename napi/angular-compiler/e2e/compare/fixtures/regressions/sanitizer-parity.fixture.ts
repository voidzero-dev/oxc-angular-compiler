/**
 * Regression: sanitizers ngtsc emits.
 *
 * Two-way bindings, host bindings checked against every concrete host,
 * MathML hrefs, namespaced elements falling back to the HTML schema and
 * iframe `credentialless`.
 */
import type { Fixture } from '../types.js'

function component(name: string, template: string): Fixture {
  const className = `${name.replace(/(^|-)(\w)/g, (_, _d, c) => c.toUpperCase())}Component`
  return {
    name,
    category: 'regressions',
    description: name,
    className,
    type: 'full-transform',
    sourceCode: `
import { Component } from '@angular/core';

@Component({
  selector: 'app-${name}',
  template: \`${template}\`,
})
export class ${className} {
  u = '';
}
    `.trim(),
    expectedFeatures: [],
  }
}

function directive(name: string, selector: string, host: string): Fixture {
  const className = `${name.replace(/(^|-)(\w)/g, (_, _d, c) => c.toUpperCase())}Directive`
  return {
    name,
    category: 'regressions',
    description: name,
    className,
    type: 'full-transform',
    sourceCode: `
import { Directive } from '@angular/core';

@Directive({ selector: '${selector}', host: ${host} })
export class ${className} {
  u = '';
}
    `.trim(),
    expectedFeatures: [],
  }
}

export const fixtures: Fixture[] = [
  component('sanitize-two-way-html', `<div [(innerHTML)]="u"></div>`),
  component('sanitize-two-way-iframe', `<iframe [(src)]="u" [(srcdoc)]="u"></iframe>`),
  component('sanitize-two-way-href', `<a [(href)]="u"></a>`),
  component('sanitize-two-way-ng-template', `<ng-template [(innerHTML)]="u"></ng-template>`),
  directive(
    'sanitize-host-div',
    'div[d]',
    `{ '[href]': 'u', '[attr.src]': 'u', '[attr.srcdoc]': 'u', '[attr.sandbox]': 'u' }`,
  ),
  directive('sanitize-host-img', 'img[d]', `{ '[src]': 'u' }`),
  directive(
    'sanitize-host-element',
    'my-el',
    `{ '[href]': 'u', '[innerHTML]': 'u', '[attr.action]': 'u', '[title]': 'u' }`,
  ),
  directive('sanitize-host-anchor', 'a[d]', `{ '[href]': 'u' }`),
  directive('sanitize-host-svg', ':svg:rect', `{ '[attr.href]': 'u' }`),
  component(
    'sanitize-mathml-href',
    `<math><a [attr.href]="u"></a><mfenced [attr.href]="u"></mfenced><menclose [attr.xlink:href]="u"></menclose><foo [attr.href]="u"></foo></math>`,
  ),
  component(
    'sanitize-svg-html-fallback',
    `<svg><iframe [src]="u"></iframe><object [data]="u"></object><iframe [srcdoc]="u"></iframe><iframe [attr.sandbox]="u"></iframe><form [action]="u"></form><area [attr.href]="u"/></svg>`,
  ),
  component('sanitize-credentialless', `<iframe [attr.credentialless]="u"></iframe>`),
]

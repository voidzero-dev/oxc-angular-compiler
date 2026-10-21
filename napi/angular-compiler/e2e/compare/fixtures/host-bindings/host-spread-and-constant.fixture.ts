/**
 * `host` metadata that is not a plain literal: a spread of an object, or a host
 * that is itself a constant. ngtsc evaluates the expression statically, so each
 * compiles to the same bindings as the entries written out, in the same order.
 */
import type { Fixture } from '../types.js'

const ALL = `{ '(click)': 'go()', '[title]': 't', '[class.x]': 'on', '[style.width]': 'w', '[attr.aria-label]': 't', role: 'button', tabindex: '0', 'data-kind': 'x', class: 'a b', style: 'color: red' }`

function host(
  name: string,
  description: string,
  preamble: string,
  expression: string,
  decorator: 'Component' | 'Directive' = 'Component',
): Fixture {
  const className = `${name.replace(/(^|-)(\w)/g, (_, _dash, c) => c.toUpperCase())}${decorator}`
  const metadata =
    decorator === 'Component'
      ? `selector: 'app-${name}', standalone: true, template: '',`
      : `selector: '[app-${name}]', standalone: true,`
  return {
    name,
    category: 'host-bindings',
    description,
    className,
    type: 'full-transform',
    sourceCode: `
import { ${decorator} } from '@angular/core';

${preamble}

@${decorator}({
  ${metadata}
  host: ${expression},
})
export class ${className} {
  t = '';
  on = true;
  w = '';
  go() {}
  other() {}
}
    `.trim(),
    expectedFeatures: [],
  }
}

export const fixtures: Fixture[] = [
  host(
    'host-spread-before',
    'Spread before other host entries',
    `const SHARED = { '(press)': 'go()' };`,
    `{ ...SHARED, accessibilityRole: 'button' }`,
  ),
  host(
    'host-spread-after',
    'Spread after other host entries',
    `const SHARED = { '(press)': 'go()' };`,
    `{ accessibilityRole: 'button', ...SHARED }`,
  ),
  host(
    'host-spread-overridden',
    'An entry after the spread overrides one of its keys',
    `const SHARED = { role: 'a', '(press)': 'go()' };`,
    `{ ...SHARED, role: 'b', '(press)': 'other()' }`,
  ),
  host(
    'host-spread-overrides',
    'A spread overrides an entry before it',
    `const SHARED = { role: 'a' };`,
    `{ role: 'b', ...SHARED }`,
  ),
  host(
    'host-two-spreads',
    'Two spreads',
    `const A = { '(press)': 'go()' };
const B = { '[title]': 't', role: 'x' };`,
    `{ ...A, ...B }`,
  ),
  host(
    'host-spread-of-spread',
    'A spread of a constant that itself spreads another',
    `const A = { '(press)': 'go()' };
const B = { ...A, '[title]': 't' };`,
    `{ ...B, role: 'x' }`,
  ),
  host('host-constant', 'Host that is a constant', `const HOST = { '(press)': 'go()' };`, 'HOST'),
  host(
    'host-constant-as-const',
    'Host that is a constant declared `as const`',
    `const HOST = { '(press)': 'go()', role: 'x' } as const;`,
    'HOST',
  ),
  host(
    'host-string-constant-values',
    'Host entries whose values and keys are string constants',
    `const HANDLER = 'go()';
const ROLE = 'button';
const KEY = '[title]';`,
    `{ '(press)': HANDLER, role: ROLE, [KEY]: 't' }`,
  ),
  host(
    'host-all-kinds-spread',
    'Every kind of key through a spread',
    `const ALL = ${ALL};`,
    '{ ...ALL }',
  ),
  host(
    'host-all-kinds-constant',
    'Every kind of key through a constant',
    `const ALL = ${ALL};`,
    'ALL',
  ),
  host(
    'host-directive-all-kinds-spread',
    'Every kind of key through a spread, on a directive',
    `const ALL = ${ALL};`,
    '{ ...ALL }',
    'Directive',
  ),
  host(
    'host-directive-all-kinds-constant',
    'Every kind of key through a constant, on a directive',
    `const ALL = ${ALL};`,
    'ALL',
    'Directive',
  ),
  host(
    'host-directive-spread',
    'Spread and an override on a directive',
    `const SHARED = { '(click)': 'go()', '[title]': 't', class: 'a' };`,
    `{ ...SHARED, role: 'button', class: 'b' }`,
    'Directive',
  ),
  host(
    'host-other-resolvable-forms',
    'A `let`, a function call, a member access and a conditional spread',
    `let BASE = { '(press)': 'go()' };
function make() { return { '[title]': 't' }; }
const C = { more: { '[class.x]': 'on' } };
const DEV = true;
const EXTRA = { role: 'x' };`,
    `{ ...BASE, ...make(), ...C.more, ...(DEV ? EXTRA : {}) }`,
  ),
]

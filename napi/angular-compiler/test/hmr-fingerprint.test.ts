import { describe, expect, it } from 'vitest'

import { componentHmrFingerprint, transformAngularFileSync } from '../index.js'

// The vite plugin decides HMR versus full reload by comparing these
// fingerprints across a save. The cases the old JS decorator lexer covered
// (comments, strings, keys, escapes, ...) are pinned in Rust
// (crates/oxc_angular_compiler/tests/hmr_fingerprint_test.rs); this checks the
// binding and the shapes that matter most to the plugin.

const SOURCE = `import { Component } from '@angular/core';

@Component({
  selector: 'app-first',
  // don't read this as a string
  template: '<div>First</div>',
  styles: ['.first { color: red; }', \`.a[data-x="]"] {}\`],
})
export class FirstComponent {
  value = 1
}

@Component({
  selector: 'app-second',
  providers: [{ provide: 'P', useValue: /['"\`)}\\]]/ }],
  template: \`<span>{{ 'Second' }}</span>\`,
})
export class SecondComponent {}
`

const CLASSES = ['FirstComponent', 'SecondComponent']

const fingerprint = (source: string, classes: string[] = CLASSES) =>
  componentHmrFingerprint(source, 'app.component.ts', classes).fingerprint

describe('componentHmrFingerprint', () => {
  it('reads every named component', () => {
    expect(componentHmrFingerprint(SOURCE, 'app.component.ts', CLASSES).components).toEqual(CLASSES)
  })

  it.each([
    ['one template', SOURCE.replace('<div>First</div>', '<div>Changed</div>')],
    ['a template literal', SOURCE.replace("{{ 'Second' }}", '{{ 2 }}')],
    ['a styles entry', SOURCE.replace('color: red', 'color: blue')],
    ['the styles array', SOURCE.replace(/styles: \[.*\]/, "styles: ['.only {}']")],
    ['both components', SOURCE.replace('First</div>', 'One</div>').replace('Second', 'Two')],
  ])('keeps the fingerprint for an edit to %s', (_what, edited) => {
    expect(fingerprint(edited)).toBe(fingerprint(SOURCE))
  })

  it.each([
    ['a class member', SOURCE.replace('value = 1', 'value = 2')],
    ['the selector', SOURCE.replace("'app-first'", "'app-one'")],
    ['a comment', SOURCE.replace("don't read", 'do not read')],
    ['the regex literal', SOURCE.replace('/[\'"`)}\\]]/', '/x/')],
    ['templateUrl', SOURCE.replace("template: '<div>First</div>'", "templateUrl: './a.html'")],
  ])('changes the fingerprint for an edit to %s', (_what, edited) => {
    expect(fingerprint(edited)).not.toBe(fingerprint(SOURCE))
  })

  it('reads only the named classes', () => {
    const edited = SOURCE.replace('<div>First</div>', '<div>Changed</div>')
    expect(fingerprint(edited, ['SecondComponent'])).not.toBe(
      fingerprint(SOURCE, ['SecondComponent']),
    )
    expect(fingerprint(edited, [])).not.toBe(fingerprint(SOURCE, []))
  })

  it('leaves nothing out of a source with syntax errors', () => {
    const broken = SOURCE.replace('<div>First</div>', "<div>it's</div>")
    expect(componentHmrFingerprint(broken, 'app.component.ts', CLASSES).components).toEqual([])
    expect(fingerprint(broken)).toBe(fingerprint(broken, []))
  })

  it('equals the fingerprint transformAngularFile reports', () => {
    const result = transformAngularFileSync(SOURCE, 'app.component.ts', { hmr: true })
    expect(result.errors).toEqual([])
    expect(result.hmrFingerprint).toBe(fingerprint(SOURCE))

    const withoutHmr = transformAngularFileSync(SOURCE, 'app.component.ts', {})
    expect(withoutHmr.hmrFingerprint).toBeUndefined()
  })
})

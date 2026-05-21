import { describe, expect, it } from 'vitest'

import {
  emptyDelimitedRange,
  locateComponentDecorators,
  locateStylesFieldFor,
  locateTemplateStringFor,
} from '../vite-plugin/utils/decorator-fields.js'

describe('decorator-fields utils', () => {
  describe('emptyDelimitedRange', () => {
    it('empties the body of a styles array but keeps the brackets', () => {
      const src = `before styles: ['x', 'y'] after`
      const open = src.indexOf('[')
      const close = src.indexOf(']')
      expect(emptyDelimitedRange(src, [open, close])).toBe('before styles: [] after')
    })

    it('empties the body of a single-quoted template', () => {
      const src = `before template: '<p/>' after`
      const open = src.indexOf("'")
      const close = src.lastIndexOf("'")
      expect(emptyDelimitedRange(src, [open, close])).toBe(`before template: '' after`)
    })

    it('empties the body of a double-quoted template', () => {
      const src = `before template: "<p/>" after`
      const open = src.indexOf('"')
      const close = src.lastIndexOf('"')
      expect(emptyDelimitedRange(src, [open, close])).toBe(`before template: "" after`)
    })

    it('empties the body of a template literal', () => {
      const src = 'before template: `<p/>` after'
      const open = src.indexOf('`')
      const close = src.lastIndexOf('`')
      expect(emptyDelimitedRange(src, [open, close])).toBe('before template: `` after')
    })

    it('is a no-op when the range already wraps an empty body', () => {
      const src = `x [] y`
      expect(emptyDelimitedRange(src, [2, 3])).toBe(src)
    })
  })

  describe('locateComponentDecorators', () => {
    it('returns [] when the source has no @Component decorator', () => {
      expect(locateComponentDecorators(`export class Foo {}`)).toEqual([])
    })

    it('returns [] when @Component is present but no class follows', () => {
      // No class declared at all — we can't pair the decorator to a name.
      const src = `@Component({ selector: 'x' })`
      expect(locateComponentDecorators(src)).toEqual([])
    })

    it('returns a single entry for a single-component file', () => {
      const src = `@Component({ selector: 'x' })\nexport class FooComponent {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('FooComponent')
      // argsRange covers `(...)` inclusive
      expect(src[out[0].argsRange[0]]).toBe('(')
      expect(src[out[0].argsRange[1]]).toBe(')')
    })

    it('returns one entry per @Component in a multi-component file', () => {
      const src = `
        import { Component } from '@angular/core';
        @Component({ selector: 'app-first', template: '<div>First</div>' })
        export class FirstComponent {}
        @Component({ selector: 'app-second', template: '<span>Second</span>' })
        export class SecondComponent {}
      `
      const out = locateComponentDecorators(src)
      expect(out.map((d) => d.className)).toEqual(['FirstComponent', 'SecondComponent'])
      // each argsRange must enclose its own args (the inner JSON literal)
      expect(src.slice(...out[0].argsRange)).toContain('app-first')
      expect(src.slice(...out[1].argsRange)).toContain('app-second')
    })

    it('handles plain `class Foo` (no `export`)', () => {
      const src = `@Component({ template: '' })\nclass Foo {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('handles `export default class Foo`', () => {
      const src = `@Component({ template: '' })\nexport default class Foo {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('handles `export abstract class Foo`', () => {
      const src = `@Component({ template: '' })\nexport abstract class Foo {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('handles extra decorators between @Component(...) and class', () => {
      const src = `@Component({ template: '' })\n@Inject() @Other()\nexport class Foo {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('handles a class with generics, extends, and implements', () => {
      const src = `@Component({ template: '' })\nexport class Foo<T extends Bar, U> extends Base<T> implements Baz<U> {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('handles class names that start with `$` or `_`', () => {
      const src = `@Component({ template: '' })\nclass $Foo {}\n@Component({ template: '' })\nclass _Bar {}`
      const out = locateComponentDecorators(src)
      expect(out.map((d) => d.className)).toEqual(['$Foo', '_Bar'])
    })

    it('skips an anonymous default-exported component (no identifier to pair)', () => {
      // `export default class { ... }` has no name. The decorator can't be
      // matched to a className → entry is skipped (HMR can't address it).
      const src = `@Component({ template: '' })\nexport default class {}`
      expect(locateComponentDecorators(src)).toEqual([])
    })

    it('does not pair an @Component with a class that belongs to a later decorator', () => {
      // The first @Component has no class before the next @Component (which has
      // its own class). The first entry should be SKIPPED, not paired with Bar.
      // (Note: comments aren't parsed away, so this fixture deliberately omits
      // the word `class` from the dangling region.)
      const src = `
        @Component({ template: '' })
        @Component({ template: '' })
        class Bar {}
      `
      const out = locateComponentDecorators(src)
      expect(out.map((d) => d.className)).toEqual(['Bar'])
    })

    it('does not pair when the next class follows another @Component', () => {
      // Same idea: the first @Component is dangling.
      const src = `
        @Component({ template: '' })
        @Component({ template: '' })
        class A {}
        @Component({ template: '' })
        class B {}
      `
      const out = locateComponentDecorators(src)
      expect(out.map((d) => d.className)).toEqual(['A', 'B'])
    })
  })

  describe('locateStylesFieldFor', () => {
    const multi = `
      @Component({ selector: 'a', styles: ['.first {}'] })
      export class FirstComponent {}
      @Component({ selector: 'b', styles: ['.second {}'] })
      export class SecondComponent {}
    `

    it('returns null when className matches no decorator', () => {
      expect(locateStylesFieldFor(multi, 'Nope')).toBeNull()
    })

    it('returns null when the named component has no styles field', () => {
      const src = `@Component({ template: '<p/>' })\nexport class Foo {}`
      expect(locateStylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('returns the FirstComponent styles range when asked for FirstComponent', () => {
      const range = locateStylesFieldFor(multi, 'FirstComponent')!
      expect(multi.slice(range[0], range[1] + 1)).toBe(`['.first {}']`)
    })

    it('returns the SecondComponent styles range when asked for SecondComponent', () => {
      const range = locateStylesFieldFor(multi, 'SecondComponent')!
      expect(multi.slice(range[0], range[1] + 1)).toBe(`['.second {}']`)
    })

    it('supports the bare-string styles form per component', () => {
      const src = `
        @Component({ styles: '.first {}' })
        export class FirstComponent {}
        @Component({ styles: '.second {}' })
        export class SecondComponent {}
      `
      const range = locateStylesFieldFor(src, 'SecondComponent')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'.second {}'`)
    })

    // The next four guard against false-matches: a `styles:` key occurring
    // inside another field's string/template literal must not be picked up.
    it('ignores `styles:` text inside a template literal that precedes the real styles', () => {
      const src =
        "@Component({ template: `<pre>const cfg = { styles: ['fake'] }</pre>`, styles: ['real'] })\nexport class Foo {}"
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('returns null when the only `styles:` text in the args is inside a template literal', () => {
      const src = "@Component({ template: `<pre>{ styles: ['fake'] }</pre>` })\nexport class Bar {}"
      expect(locateStylesFieldFor(src, 'Bar')).toBeNull()
    })

    it("ignores `styles:` inside a `${...}` interpolation's nested object literal", () => {
      // `${ ... { styles: [...] } ... }` inside a template literal must not
      // be treated as a top-level @Component metadata property.
      const src =
        "@Component({ template: `${doThing({ styles: ['fake'] })}`, styles: ['real'] })\nexport class Baz {}"
      const range = locateStylesFieldFor(src, 'Baz')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('ignores `styles:` inside a nested non-metadata object literal', () => {
      // `metadata: { styles: ['nested'] }` is not the component's `styles`
      // field; only top-level properties of the @Component argument count.
      const src = `@Component({ host: { '[styles]': 'expr', styles: 'irrelevant' }, styles: ['real'] })\nexport class Qux {}`
      const range = locateStylesFieldFor(src, 'Qux')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })
  })

  describe('locateTemplateStringFor', () => {
    const multi = `
      @Component({ selector: 'a', template: '<first/>' })
      export class FirstComponent {}
      @Component({ selector: 'b', template: '<second/>' })
      export class SecondComponent {}
    `

    it('returns null when className matches no decorator', () => {
      expect(locateTemplateStringFor(multi, 'Nope')).toBeNull()
    })

    it('returns null when the named component has no template field', () => {
      const src = `@Component({ styles: [] })\nexport class Foo {}`
      expect(locateTemplateStringFor(src, 'Foo')).toBeNull()
    })

    it('returns the FirstComponent template range when asked for FirstComponent', () => {
      const range = locateTemplateStringFor(multi, 'FirstComponent')!
      expect(multi.slice(range[0], range[1] + 1)).toBe(`'<first/>'`)
    })

    it('returns the SecondComponent template range when asked for SecondComponent', () => {
      const range = locateTemplateStringFor(multi, 'SecondComponent')!
      expect(multi.slice(range[0], range[1] + 1)).toBe(`'<second/>'`)
    })

    it("ignores `template:` text appearing inside another field's string literal", () => {
      // The `styles` array contains a string with literal `template:` text;
      // the real `template:` field comes after. The naive regex would match
      // the inner one first.
      const src = `@Component({ styles: ['/* template: "fake" */'], template: '<real/>' })\nexport class Foo {}`
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'<real/>'`)
    })
  })

  // -----------------------------------------------------------------
  // Comment-aware scanning. Without this, the walker treats `'` in a
  // `// don't ...` line comment as opening a string literal that never
  // closes (real field missed), and a `// styles: [...]` line comment
  // or `/* styles: [...] */` block comment as a real field (wrong
  // range returned).
  // -----------------------------------------------------------------
  describe('comment handling in @Component args', () => {
    it('does not get stuck on an apostrophe inside a line comment', () => {
      const src = `@Component({
  // I'm setting the styles below
  styles: ['real']
})
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('does not get stuck on apostrophes inside a block comment', () => {
      const src = `@Component({
  /* It's important: don't use these */
  styles: ['real']
})
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('ignores `styles:` inside a line comment', () => {
      const src = `@Component({
  // styles: ['fake'],
  styles: ['real']
})
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('ignores `styles:` inside a block comment', () => {
      const src = `@Component({
  /* styles: ['fake'] */
  styles: ['real']
})
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('ignores `template:` inside a block comment', () => {
      const src = `@Component({
  /* template: '<fake/>' */
  template: '<real/>'
})
class Foo {}`
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'<real/>'`)
    })

    it('returns null when the only `styles:` is inside a comment', () => {
      const src = `@Component({
  // styles: ['fake']
  selector: 'app-foo'
})
class Foo {}`
      expect(locateStylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('handles a block comment spanning multiple lines', () => {
      const src = `@Component({
  /*
   * styles: ['fake-line-1']
   * styles: ['fake-line-2']
   */
  styles: ['real']
})
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('handles a comment between @Component(...) and the class declaration', () => {
      const src = `@Component({ styles: ['x'] })
// I'm decorating this class
export class Foo {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('does NOT treat `//` inside a string as a comment', () => {
      // `'http://x'` is a URL in a value, not a comment.
      const src = `@Component({ template: 'http://x', styles: ['real'] })
class Foo {}`
      const tRange = locateTemplateStringFor(src, 'Foo')!
      const sRange = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(tRange[0], tRange[1] + 1)).toBe(`'http://x'`)
      expect(src.slice(sRange[0], sRange[1] + 1)).toBe(`['real']`)
    })

    it('does NOT treat `/*` inside a string as a block comment', () => {
      const src = `@Component({ template: '/* not a comment */', styles: ['real'] })
class Foo {}`
      const sRange = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(sRange[0], sRange[1] + 1)).toBe(`['real']`)
    })
  })

  // -----------------------------------------------------------------
  // Regression coverage for things that *could* look like a decorator
  // or field but must not be picked up: text inside other strings, in
  // other decorators, in class member bodies, etc.
  // -----------------------------------------------------------------
  describe('robust against decoy tokens elsewhere in the source', () => {
    it('ignores `@Component(...)` example inside a JSDoc block before the real decorator', () => {
      const src = `/**
 * Usage example:
 *   @Component({ template: 'fake' })
 *   class Example {}
 */
@Component({ template: '<real/>' })
export class Foo {}`
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'<real/>'`)
    })

    it('ignores `@Component(...)` text inside a string literal preceding the real decorator', () => {
      const src = `const docs = "use @Component({ template: 'fake' }) to declare"
@Component({ template: '<real/>' })
class Foo {}`
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'<real/>'`)
    })

    it('ignores `@Component(...)` text inside a backtick template preceding the real decorator', () => {
      const src =
        "`Use @Component({ template: 'fake' })`\n@Component({ template: '<real/>' })\nclass Foo {}"
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'<real/>'`)
    })

    it('does not get confused by a `template:` literal that mentions the word "styles:"', () => {
      const src = `@Component({ template: 'styles: ["fake"]', styles: ['real'] })
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('handles unbalanced braces or brackets inside template string content', () => {
      const src = `@Component({ template: 'has { and ] literally', styles: ['real'] })
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('treats CRLF line endings the same as LF', () => {
      const src = `@Component({\r\n  // a comment with an apostrophe: I'm here\r\n  styles: ['real']\r\n})\r\nclass Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('handles `...spread` followed by a real `styles:` field', () => {
      const src = `const base = { selector: 'app' }
@Component({ ...base, styles: ['real'] })
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('handles a selector value that contains parens', () => {
      const src = `@Component({ selector: 'foo(bar)', styles: ['real'] })
class Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['real']`)
    })

    it('ignores a class member method named `Component`', () => {
      const src = `@Component({ template: '<real/>' })
class Foo {
  Component(x: number) { return x; }
}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Foo')
    })

    it('handles a helper function whose body contains an "@Component" string between two real components', () => {
      const src = `@Component({ template: '<a/>' })
class First {}
const helper = () => '@Component({...})'
@Component({ template: '<b/>' })
class Second {}`
      const out = locateComponentDecorators(src)
      expect(out.map((d) => d.className)).toEqual(['First', 'Second'])
      const fRange = locateTemplateStringFor(src, 'First')!
      const sRange = locateTemplateStringFor(src, 'Second')!
      expect(src.slice(fRange[0], fRange[1] + 1)).toBe(`'<a/>'`)
      expect(src.slice(sRange[0], sRange[1] + 1)).toBe(`'<b/>'`)
    })

    it('handles literal `$` followed by `${...}` interpolation in a template literal', () => {
      const src = '@Component({ template: `cost $5 or $${price}` })\nclass Foo {}'
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe('`cost $5 or $${price}`')
    })

    it('coexists with other class-level decorators like @SignalComponent', () => {
      const src = `@SignalComponent({ template: 'sig' })
@Component({ template: '<real/>' })
class Foo {}`
      // Only @Component is recognized; @SignalComponent is ignored entirely.
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      const range = locateTemplateStringFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'<real/>'`)
    })
  })

  describe('Unicode class identifiers', () => {
    it('captures a class name containing non-ASCII letters', () => {
      const src = `@Component({ styles: ['x'] })\nclass Café {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('Café')
    })

    it('captures CJK class names', () => {
      const src = `@Component({ styles: ['x'] })\nclass 组件 {}`
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('组件')
    })
  })
})

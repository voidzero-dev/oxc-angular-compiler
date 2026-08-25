import { describe, expect, it } from 'vitest'

import {
  emptyDelimitedRange,
  locateComponentDecorators,
  locateStyleFieldsFor,
  locateStyleUrlFor,
  locateStyleUrlsFor,
  locateStylesFieldFor,
  locateTemplateStringFor,
  locateTemplateUrlFor,
  readStringLiterals,
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

    it('ignores a commented-out decorator that follows the real one', () => {
      // The phantom sits between the real decorator and the class. Pairing
      // the class with it would read the stale metadata, not merely miss it.
      const src = [
        `@Component({ selector: 'x', styleUrls: ['./real.css'] })`,
        `// @Component({ styleUrl: './old.css' })`,
        `export class FooComponent {}`,
      ].join('\n')
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('FooComponent')
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'FooComponent')!).literals).toEqual([
        './real.css',
      ])
    })

    it('ignores a decorator inside a block comment that follows the real one', () => {
      const src = [
        `@Component({ selector: 'x', styleUrls: ['./real.css'] })`,
        `/* @Component({ styleUrl: './old.css' }) */`,
        `export class FooComponent {}`,
      ].join('\n')
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'FooComponent')!).literals).toEqual([
        './real.css',
      ])
    })

    it('ignores a commented-out decorator that precedes the real one', () => {
      const src = [
        `// @Component({ styleUrl: './old.css' })`,
        `@Component({ selector: 'x', styleUrls: ['./real.css'] })`,
        `export class FooComponent {}`,
      ].join('\n')
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'FooComponent')!).literals).toEqual([
        './real.css',
      ])
    })

    it('ignores a decorator written inside a string literal', () => {
      const src = [
        `const doc = 'see @Component({ styleUrl: "./str.css" }) for details';`,
        `@Component({ selector: 'x', styleUrls: ['./real.css'] })`,
        `export class FooComponent {}`,
      ].join('\n')
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'FooComponent')!).literals).toEqual([
        './real.css',
      ])
    })

    it('pairs each class with its own decorator when a phantom sits between them', () => {
      const src = [
        `@Component({ selector: 'a', styleUrls: ['./a.css'] })`,
        `// @Component({ styleUrl: './fake.css' })`,
        `export class AComponent {}`,
        `@Component({ selector: 'b', styleUrls: ['./b.css'] })`,
        `export class BComponent {}`,
      ].join('\n')
      expect(locateComponentDecorators(src).map((d) => d.className)).toEqual([
        'AComponent',
        'BComponent',
      ])
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'AComponent')!).literals).toEqual([
        './a.css',
      ])
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'BComponent')!).literals).toEqual([
        './b.css',
      ])
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

    it('does not match a `templateUrl:` field as `template:`', () => {
      const src = `@Component({ templateUrl: './foo.html' })\nexport class Foo {}`
      expect(locateTemplateStringFor(src, 'Foo')).toBeNull()
    })
  })

  describe('locateTemplateUrlFor', () => {
    const multi = `
      @Component({ selector: 'a', templateUrl: './first.html' })
      export class FirstComponent {}
      @Component({ selector: 'b', templateUrl: './second.html' })
      export class SecondComponent {}
    `

    it('returns null when className matches no decorator', () => {
      expect(locateTemplateUrlFor(multi, 'Nope')).toBeNull()
    })

    it('returns null when the named component has no templateUrl field', () => {
      const src = `@Component({ template: '<p/>' })\nexport class Foo {}`
      expect(locateTemplateUrlFor(src, 'Foo')).toBeNull()
    })

    it('returns each component its own templateUrl range in a multi-component file', () => {
      const first = locateTemplateUrlFor(multi, 'FirstComponent')!
      const second = locateTemplateUrlFor(multi, 'SecondComponent')!
      expect(multi.slice(first[0], first[1] + 1)).toBe(`'./first.html'`)
      expect(multi.slice(second[0], second[1] + 1)).toBe(`'./second.html'`)
    })

    it('does not match an inline `template:` field as `templateUrl:`', () => {
      const src = `@Component({ template: '<p>templateUrl: fake</p>' })\nexport class Foo {}`
      expect(locateTemplateUrlFor(src, 'Foo')).toBeNull()
    })

    it('finds templateUrl when the decorator also has an inline template field', () => {
      const src = `@Component({ template: '<p/>', templateUrl: './real.html' })\nexport class Foo {}`
      const range = locateTemplateUrlFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'./real.html'`)
    })
  })

  describe('locateStyleUrlsFor / locateStyleUrlFor', () => {
    const multi = `
      @Component({ selector: 'a', styleUrls: ['./first.css'] })
      export class FirstComponent {}
      @Component({ selector: 'b', styleUrls: ['./second.css', './extra.css'] })
      export class SecondComponent {}
    `

    it('returns null when className matches no decorator', () => {
      expect(locateStyleUrlsFor(multi, 'Nope')).toBeNull()
      expect(locateStyleUrlFor(multi, 'Nope')).toBeNull()
    })

    it('returns each component its own styleUrls range in a multi-component file', () => {
      const first = locateStyleUrlsFor(multi, 'FirstComponent')!
      const second = locateStyleUrlsFor(multi, 'SecondComponent')!
      expect(multi.slice(first[0], first[1] + 1)).toBe(`['./first.css']`)
      expect(multi.slice(second[0], second[1] + 1)).toBe(`['./second.css', './extra.css']`)
    })

    it('locates the singular `styleUrl:` string form', () => {
      const src = `@Component({ styleUrl: './solo.css' })\nexport class Foo {}`
      const range = locateStyleUrlFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`'./solo.css'`)
    })

    // The four cross-match guards: `styleUrl`, `styleUrls` and `styles` are
    // distinct fields and must never resolve to one another.
    it('does not match `styleUrls:` when looking for the singular `styleUrl:`', () => {
      const src = `@Component({ styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(locateStyleUrlFor(src, 'Foo')).toBeNull()
    })

    it('does not match the singular `styleUrl:` when looking for `styleUrls:`', () => {
      const src = `@Component({ styleUrl: './a.css' })\nexport class Foo {}`
      expect(locateStyleUrlsFor(src, 'Foo')).toBeNull()
    })

    it('does not match an inline `styles:` field as either url field', () => {
      const src = `@Component({ styles: ['.a { color: red }'] })\nexport class Foo {}`
      expect(locateStyleUrlsFor(src, 'Foo')).toBeNull()
      expect(locateStyleUrlFor(src, 'Foo')).toBeNull()
    })

    it('does not match either url field as the inline `styles:` field', () => {
      const urls = `@Component({ styleUrls: ['./a.css'] })\nexport class Foo {}`
      const url = `@Component({ styleUrl: './a.css' })\nexport class Foo {}`
      expect(locateStylesFieldFor(urls, 'Foo')).toBeNull()
      expect(locateStylesFieldFor(url, 'Foo')).toBeNull()
    })

    it('finds styleUrls when the decorator also has an inline styles field', () => {
      const src = `@Component({ styles: ['.x{}'], styleUrls: ['./real.css'] })\nexport class Foo {}`
      const range = locateStyleUrlsFor(src, 'Foo')!
      expect(src.slice(range[0], range[1] + 1)).toBe(`['./real.css']`)
    })
  })

  // The endpoint needs three answers, not two: a class with no styles must be
  // served none, while a class whose styles cannot be read must fall back to
  // the file-level list. Only a null return distinguishes them.
  describe('locateStyleFieldsFor', () => {
    // Read the literals out of a located field, for the tests that care
    // about the value rather than the classification.
    const literalsIn = (src: string, field: { kind: string; range?: [number, number] }) => {
      expect(field.kind).toBe('literal')
      return readStringLiterals(src, (field as { range: [number, number] }).range).literals
    }

    it('returns null when className matches no decorator', () => {
      const src = `@Component({ styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Nope')).toBeNull()
    })

    it('reports both fields absent for a class that declares no styles', () => {
      const src = `@Component({ template: '<p></p>' })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')).toEqual({
        urls: { kind: 'absent' },
        inline: { kind: 'absent' },
      })
    })

    it('reports a field whose value has no literal shape as unreadable', () => {
      const src = `@Component({ styleUrls: STYLE_URLS })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
    })

    it('reports a singular `styleUrl` identifier value as unreadable, not absent', () => {
      // The regression this guards: reporting it absent tells the caller
      // "this class has no styles", which strips the component's CSS.
      const src = `@Component({ styleUrl: STYLE_URL })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
    })

    it('reports inline and url ranges together when a class declares both', () => {
      const src = `@Component({ styles: ['.x{}'], styleUrls: ['./real.css'] })\nexport class Foo {}`
      const fields = locateStyleFieldsFor(src, 'Foo')!
      expect(literalsIn(src, fields.inline)).toEqual(['.x{}'])
      expect(literalsIn(src, fields.urls)).toEqual(['./real.css'])
    })

    it('falls back to the singular `styleUrl` for the urls member', () => {
      const src = `@Component({ styleUrl: './solo.css' })\nexport class Foo {}`
      const fields = locateStyleFieldsFor(src, 'Foo')!
      expect(literalsIn(src, fields.urls)).toEqual(['./solo.css'])
      expect(fields.inline).toEqual({ kind: 'absent' })
    })

    it('reports the urls member unreadable when only the singular form is unreadable', () => {
      const src = `@Component({ styleUrl: STYLE_URL, styles: ['.x{}'] })\nexport class Foo {}`
      const fields = locateStyleFieldsFor(src, 'Foo')!
      expect(fields.urls).toEqual({ kind: 'unreadable' })
      expect(literalsIn(src, fields.inline)).toEqual(['.x{}'])
    })

    it('resolves each class separately in a multi-component file', () => {
      const src = `
        @Component({ selector: 'a', styleUrls: ['./a.css'] })
        export class StyledComponent {}
        @Component({ selector: 'b', template: '<p></p>' })
        export class BareComponent {}
      `
      const styled = locateStyleFieldsFor(src, 'StyledComponent')!
      expect(literalsIn(src, styled.urls)).toEqual(['./a.css'])
      expect(locateStyleFieldsFor(src, 'BareComponent')).toEqual({
        urls: { kind: 'absent' },
        inline: { kind: 'absent' },
      })
    })

    // Quoted keys are valid TS and the Rust extractor resolves them
    // (verified: `'styleUrls'` and `"styleUrls"` both report their URL).
    // Reading them as absent tells the caller the class declares no
    // styles, which strips the component's CSS.
    it('reads a single-quoted key the same as the bare form', () => {
      const src = `@Component({ 'styleUrls': ['./a.css'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    it('reads a double-quoted key the same as the bare form', () => {
      const src = `@Component({ "styleUrls": ['./a.css'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    it('reads a quoted singular `styleUrl` key', () => {
      const src = `@Component({ 'styleUrl': './solo.css' })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./solo.css'])
    })

    it('reads a quoted inline `styles` key', () => {
      const src = `@Component({ 'styles': ['.x{}'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.inline)).toEqual(['.x{}'])
    })

    it('keeps the cross-match guards for quoted keys', () => {
      const urls = `@Component({ 'styleUrls': ['./a.css'] })\nexport class Foo {}`
      expect(locateStyleFieldsFor(urls, 'Foo')!.inline).toEqual({ kind: 'absent' })
      const inline = `@Component({ 'styles': ['.x{}'] })\nexport class Foo {}`
      expect(locateStyleFieldsFor(inline, 'Foo')!.urls).toEqual({ kind: 'absent' })
    })

    // A computed key hides the field name from this scan, but the Rust
    // extractor resolves it (verified: `[K]: ['./computed.css']` reports
    // the URL). "Absent" would be a lie, so the whole classification is
    // unknown and the caller falls back.
    it('reports both fields unreadable when a computed key is present', () => {
      const src = `@Component({ [K]: ['./a.css'] })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')).toEqual({
        urls: { kind: 'unreadable' },
        inline: { kind: 'unreadable' },
      })
    })

    it('reports both fields unreadable for a quoted key whose escape is malformed', () => {
      // A decodable escape is resolved instead — see the `escaped keys`
      // block. Only an escape this scan cannot decode leaves the field name
      // unknown, and then "absent" would be a guess rather than an answer.
      const src = `@Component({ 'style\\u00ZZrls': ['./a.css'] })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')).toEqual({
        urls: { kind: 'unreadable' },
        inline: { kind: 'unreadable' },
      })
    })

    it('does not promote a readable field to unreadable because of a computed key', () => {
      // The visible field is still exactly what it says; only the fields
      // this scan cannot see are unknown.
      const src = `@Component({ [K]: 1, styleUrls: ['./a.css'] })\nexport class Foo {}`
      const fields = locateStyleFieldsFor(src, 'Foo')!
      expect(literalsIn(src, fields.urls)).toEqual(['./a.css'])
      expect(fields.inline).toEqual({ kind: 'unreadable' })
    })

    // A spread is the one unreadable form the Rust extractor ALSO drops
    // (verified: `...BASE` reports no styleUrls). Both sides see nothing,
    // so "absent" matches what the compiled component gets; falling back
    // would hand this class its siblings' CSS.
    it('leaves fields absent for a spread, which the compiler also drops', () => {
      const src = `@Component({ ...BASE })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')).toEqual({
        urls: { kind: 'absent' },
        inline: { kind: 'absent' },
      })
    })

    // Shorthand style fields. Measured against the Rust extractor, which
    // resolves a same-file string constant behind the singular `styleUrl`
    // but drops the array-valued forms — so the two need opposite answers,
    // for the same reason the spread above stays absent: match what the
    // compiled component actually ends up with.
    it('reports a shorthand singular `styleUrl` as unreadable, not absent', () => {
      const src = `@Component({ template: '<p/>', styleUrl })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
    })

    it('leaves a shorthand `styleUrls` absent, which the compiler also drops', () => {
      const src = `@Component({ template: '<p/>', styleUrls })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
    })

    it('leaves a shorthand inline `styles` absent, which the compiler also drops', () => {
      const src = `@Component({ template: '<p/>', styles })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.inline).toEqual({ kind: 'absent' })
    })

    it('does not let an unrelated shorthand degrade a readable field', () => {
      const src = `@Component({ selector, styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    // The trap in accepting a bare key: an identifier used as a VALUE is not
    // a shorthand property, and reading it as one would fall back on a field
    // that is right there and readable.
    it('does not mistake a style-named identifier used as a value for a shorthand', () => {
      const src = `@Component({ selector: styleUrl, styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    it('does not let an unrelated method degrade a readable field', () => {
      const src = `@Component({ foo() { return 1 }, styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    // A method or accessor named like a style field is not a style field:
    // the compiler reads no styles from it either.
    it('leaves a method named like a style field absent', () => {
      const src = `@Component({ styleUrls() { return ['./a.css'] } })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
    })

    it('leaves a getter named like a style field absent', () => {
      const src = `@Component({ get styleUrl() { return './a.css' } })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
    })

    it('reads the real field past a setter named like a style field', () => {
      const src = `@Component({ set styleUrl(v) {}, styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    it('does not read a style field nested in a deeper object', () => {
      const src = `@Component({ data: { styleUrls: ['./deep.css'] } })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
    })

    it('reads a field followed by a trailing comma', () => {
      const src = `@Component({ styleUrls: ['./a.css'], })\nexport class Foo {}`
      expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
    })

    it('reads a shorthand style field that closes the object', () => {
      // No trailing comma — the key is bounded by `}` rather than `,`.
      const src = `@Component({ styleUrl })\nexport class Foo {}`
      expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
    })

    // A value that merely STARTS with a literal does not denote it. Every
    // form below was measured against the Rust extractor: it reports no
    // styleUrls and compiles no styles for any of them. Returning the
    // leading literal would hand the endpoint a stylesheet the component
    // never had, and — being a confident answer — would skip the fallback
    // that exists for exactly this case.
    describe('a literal that is only the start of a larger expression', () => {
      const urls = (field: string) =>
        locateStyleFieldsFor(
          `const SUFFIX = '.s.css';\nconst MORE: string[] = ['./m.css'];\n@Component({ template: '<p/>', ${field} })\nexport class Foo {}`,
          'Foo',
        )!.urls
      const inline = (field: string) =>
        locateStyleFieldsFor(
          `const EXTRA = '.b{}';\n@Component({ template: '<p/>', ${field} })\nexport class Foo {}`,
          'Foo',
        )!.inline

      it('rejects a singular `styleUrl` concatenated with an identifier', () => {
        expect(urls(`styleUrl: './a.css' + SUFFIX`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a singular `styleUrl` concatenated with another literal', () => {
        expect(urls(`styleUrl: './a.css' + './b.css'`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a `styleUrls` array concatenated with an identifier', () => {
        expect(urls(`styleUrls: './a.css' + SUFFIX`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects an inline `styles` value concatenated with an identifier', () => {
        expect(inline(`styles: '.a{}' + EXTRA`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a method call on a style literal', () => {
        expect(urls(`styleUrl: './a.css'.replace('a', 'b')`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a method call on a `styleUrls` array literal', () => {
        expect(urls(`styleUrls: ['./a.css'].concat(MORE)`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a method call on an inline `styles` array literal', () => {
        expect(inline(`styles: ['.a{}'].concat(MORE)`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a TypeScript `as` assertion after a style literal', () => {
        expect(urls(`styleUrl: './a.css' as string`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a TypeScript `as const` assertion after a `styleUrls` array', () => {
        expect(urls(`styleUrls: ['./a.css'] as const`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a TypeScript `as` assertion after an inline `styles` array', () => {
        expect(inline(`styles: ['.a{}'] as string[]`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a non-null assertion after a style literal', () => {
        expect(urls(`styleUrl: './a.css'!`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a `satisfies` clause after a style literal', () => {
        expect(urls(`styleUrl: './a.css' satisfies string`)).toEqual({ kind: 'unreadable' })
      })

      it('rejects a trailing expression hidden behind a comment', () => {
        expect(urls(`styleUrl: './a.css' /* why */ + SUFFIX`)).toEqual({ kind: 'unreadable' })
      })

      // The other side of the rule: a value the property really does end at
      // stays readable, whether a comma, the object's brace or a comment
      // closes it out.
      it.each([
        [`styleUrl: './a.css'`, `the object's closing brace`],
        [`styleUrl: './a.css',`, 'a trailing comma'],
        [`styleUrl: './a.css' /* why */`, 'a block comment then the brace'],
        [`styleUrl: './a.css' // why\n`, 'a line comment then the brace'],
        [`styleUrls: ['./a.css']`, 'an array at the closing brace'],
        [`styleUrls: ['./a.css'],`, 'an array with a trailing comma'],
      ])('still reads %j, ended by %s', (field) => {
        const src = `@Component({ template: '<p/>', ${field} })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
      })
    })

    // Escaped identifier keys. `style\u0055rls` IS `styleUrls` to the
    // TypeScript parser, and the Rust extractor resolves it (verified: it
    // reports `./x.css`). Reading it as absent told the caller the class
    // declares no styles, which strips the component's CSS. Decoding gives
    // an exact per-class answer, where falling back would serve the
    // file-level union — every sibling's CSS along with this class's own.
    describe('escaped keys', () => {
      it('decodes a \\uHHHH escape in a bare key', () => {
        const src = `@Component({ style\\u0055rls: ['./a.css'] })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
      })

      it('decodes a \\u{…} escape in a bare key', () => {
        const src = `@Component({ style\\u{55}rls: ['./a.css'] })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
      })

      it('decodes an escape at the first character of a bare key', () => {
        const src = `@Component({ \\u0073tyleUrls: ['./a.css'] })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
      })

      it('decodes an escaped singular `styleUrl` key', () => {
        const src = `@Component({ style\\u0055rl: './solo.css' })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./solo.css'])
      })

      it('decodes an escaped inline `styles` key', () => {
        const src = `@Component({ style\\u0073: ['.x{}'] })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.inline)).toEqual(['.x{}'])
      })

      it('decodes a quoted key carrying an escape', () => {
        // A quoted key is a string literal, so it decodes by string rules.
        const src = `@Component({ 'style\\u0055rls': ['./a.css'] })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
      })

      it('treats an escaped shorthand singular `styleUrl` like the plain one', () => {
        const src = `@Component({ template: '<p/>', style\\u0055rl })\nexport class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
      })

      it('leaves an escaped shorthand `styleUrls` absent, as the compiler drops it', () => {
        const src = `@Component({ template: '<p/>', style\\u0055rls })\nexport class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
      })

      it('keeps the cross-match guards for decoded keys', () => {
        const urls = `@Component({ style\\u0055rls: ['./a.css'] })\nexport class Foo {}`
        expect(locateStyleFieldsFor(urls, 'Foo')!.inline).toEqual({ kind: 'absent' })
        const singular = `@Component({ style\\u0055rl: './a.css' })\nexport class Foo {}`
        expect(locateStyleFieldsFor(singular, 'Foo')!.inline).toEqual({ kind: 'absent' })
        const inline = `@Component({ style\\u0073: ['.x{}'] })\nexport class Foo {}`
        expect(locateStyleFieldsFor(inline, 'Foo')!.urls).toEqual({ kind: 'absent' })
      })

      it('does not match a decoded key that names something else', () => {
        const src = `@Component({ style\\u0055rlsExtra: ['./a.css'] })\nexport class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
      })

      it('does not match a lookalike built from a non-ASCII letter', () => {
        // Cyrillic \u0435 in place of `e` — a different identifier, and the
        // compiler reports no styleUrls for it either.
        const src = `@Component({ styl\u0435Urls: ['./a.css'] })\nexport class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'absent' })
      })

      it('reports a malformed escape in a key as unreadable, not absent', () => {
        const src = `@Component({ style\\u00ZZrls: ['./a.css'] })\nexport class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')).toEqual({
          urls: { kind: 'unreadable' },
          inline: { kind: 'unreadable' },
        })
      })

      it('reports a \\xHH escape in a key as unreadable — illegal in an identifier', () => {
        const src = `@Component({ style\\x55rls: ['./a.css'] })\nexport class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')).toEqual({
          urls: { kind: 'unreadable' },
          inline: { kind: 'unreadable' },
        })
      })

      it('does not let an escaped unrelated key degrade a readable field', () => {
        const src = `@Component({ sel\\u0065ctor: 'a', styleUrls: ['./a.css'] })\nexport class Foo {}`
        expect(literalsIn(src, locateStyleFieldsFor(src, 'Foo')!.urls)).toEqual(['./a.css'])
      })
    })
  })

  // -----------------------------------------------------------------
  // Comment-aware scanning. Without this, the walker treats `'` in a
  // `// don't ...` line comment as opening a string literal that never
  // closes (real field missed), and a `// styles: [...]` line comment
  // or `/* styles: [...] */` block comment as a real field (wrong
  // range returned).
  //
  // `complete` reports whether every element was read. A caller acting
  // on a partial list silently drops stylesheets.
  // -----------------------------------------------------------------
  describe('readStringLiterals', () => {
    // Read the value of `styles:` from a one-component source, so these
    // exercise the same path the extractors use.
    const readOf = (value: string) => {
      const src = `@Component({ styles: ${value} })\nclass Foo {}`
      return readStringLiterals(src, locateStylesFieldFor(src, 'Foo')!)
    }
    const literalsOf = (value: string): string[] => readOf(value).literals

    it('reads every entry of an array in order', () => {
      expect(readOf(`['./a.css', './b.css']`)).toEqual({
        literals: ['./a.css', './b.css'],
        complete: true,
      })
    })

    it('reads a single-entry array', () => {
      expect(literalsOf(`['./a.css']`)).toEqual(['./a.css'])
    })

    it('reads a bare string value as one entry', () => {
      expect(readOf(`'./a.css'`)).toEqual({ literals: ['./a.css'], complete: true })
    })

    it('reads mixed quote styles, including template literals', () => {
      expect(literalsOf('[\'./a.css\', "./b.css", `./c.css`]')).toEqual([
        './a.css',
        './b.css',
        './c.css',
      ])
    })

    it('reports an empty array as complete, not unknown', () => {
      // `styles: []` is a definitive answer: this class has no styles.
      expect(readOf(`[]`)).toEqual({ literals: [], complete: true })
    })

    it('decodes hex and unicode escapes to the value they denote', () => {
      // The Rust extractor reports the cooked value; a raw read would name a
      // path that does not exist.
      expect(literalsOf(String.raw`['.\x2fa.css', '\u{2e}/b.css']`)).toEqual(['./a.css', './b.css'])
    })

    it('decodes the standard single-character escapes', () => {
      expect(literalsOf(String.raw`['a\nb\tc\rd\be\ff\vg']`)).toEqual(['a\nb\tc\rd\be\ff\vg'])
    })

    it('decodes a backslash escape to one backslash', () => {
      expect(literalsOf(String.raw`['a\\b']`)).toEqual([String.raw`a\b`])
    })

    it('decodes an unrecognized escape to the character itself', () => {
      // `\q` is a NonEscapeCharacter: it denotes `q`, which is what the Rust
      // extractor reports too.
      expect(literalsOf(String.raw`['./a\qb.css']`)).toEqual(['./aqb.css'])
    })

    it('drops a line continuation', () => {
      expect(literalsOf("['a\\\nb']")).toEqual(['ab'])
    })

    it('reports a truncated unicode escape as incomplete', () => {
      expect(readOf(String.raw`['./a\u12']`)).toEqual({ literals: [], complete: false })
    })

    it('reports a malformed hex escape as incomplete', () => {
      expect(readOf(String.raw`['./a\xZZ']`)).toEqual({ literals: [], complete: false })
    })

    it('reports a legacy octal escape as incomplete', () => {
      // Illegal in a module; guessing a value would be worse than falling back.
      expect(readOf(String.raw`['./a\101']`)).toEqual({ literals: [], complete: false })
    })

    it('leaves a literal with no escapes byte for byte unchanged', () => {
      const css = `a::before { content: "x"; } [data-x="y"] { color: red; }`
      expect(literalsOf(`['${css}']`)).toEqual([css])
    })

    it('decodes an escaped quote inside a literal', () => {
      // The literal denotes `it's.css`, which is what the Rust extractor
      // reports and what has to be resolved against the filesystem.
      expect(literalsOf(`['it\\'s.css']`)).toEqual([`it's.css`])
    })

    it('ignores an apostrophe inside a block comment before an entry', () => {
      expect(literalsOf(`[/* don't drop this */ './a.css']`)).toEqual(['./a.css'])
    })

    it('ignores an apostrophe inside a line comment before an entry', () => {
      expect(literalsOf(`[\n  // it's here\n  './a.css',\n]`)).toEqual(['./a.css'])
    })

    it('ignores a comment between two entries', () => {
      expect(literalsOf(`['./a.css', /* don't */ './b.css']`)).toEqual(['./a.css', './b.css'])
    })

    it('reports a non-literal entry as incomplete', () => {
      // The literal alongside it is not a partial answer to act on: acting
      // on it would drop the stylesheet the constant resolves to.
      expect(readOf(`[SOME_CONST, './a.css']`).complete).toBe(false)
    })

    // -----------------------------------------------------------------
    // Spread elements (`...X`) and elisions (holes).
    //
    // `extract_string_array` asks every array element for `as_expression()`,
    // and `ArrayExpressionElement::is_expression()` enumerates neither
    // `SpreadElement` nor `Elision`. Both are therefore dropped BEFORE the
    // value resolver runs, for every possible program — the const table is
    // never consulted for them. Dropping them here keeps the array complete
    // and matches what the component actually compiles to; calling it
    // unknown would hand the class its siblings' stylesheets instead.
    //
    // Measured against the Rust extractor:
    //   [...SHARED, './own.css']       -> ["./own.css"]
    //   [...SHARED]                    -> []
    //   [S, './own.css']  (S a const)  -> ["./shared.css", "./own.css"]
    //
    // Every OTHER element IS an expression the resolver may fold from
    // constants this scan cannot read, so those still mark the array
    // unknown. Same reasoning as the decorator-level spread in
    // `hasUnreadableKey`; the two now agree.
    // -----------------------------------------------------------------
    it('drops a leading spread and reads the rest, like the compiler', () => {
      expect(readOf(`[...SHARED, './a.css']`)).toEqual({
        literals: ['./a.css'],
        complete: true,
      })
    })

    it('drops a trailing spread and reads the rest', () => {
      expect(readOf(`['./a.css', ...SHARED]`)).toEqual({
        literals: ['./a.css'],
        complete: true,
      })
    })

    it('reads a spread-only array as declaring nothing', () => {
      expect(readOf(`[...SHARED]`)).toEqual({ literals: [], complete: true })
    })

    it('reads an array of two spreads as declaring nothing', () => {
      expect(readOf(`[...S1, ...S2]`)).toEqual({ literals: [], complete: true })
    })

    it('drops a spread of an inline array literal', () => {
      // The compiler drops the element whole, so a literal INSIDE the spread
      // is not part of this component's styles.
      expect(readOf(`[...['./shared.css'], './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('never leaks a literal used as a computed member inside a spread', () => {
      expect(readOf(`[...obj['./leak.css'], './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('never leaks a literal passed as a call argument inside a spread', () => {
      expect(readOf(`[...f('./leak.css'), './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('never leaks a literal inside an object literal in a spread', () => {
      expect(readOf(`[...Object.values({ k: './leak.css' }), './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('keeps a comma inside parens from ending the spread element', () => {
      expect(readOf(`[...f(a, b), './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('keeps a comma inside a nested array from ending the spread element', () => {
      expect(readOf(`[...['./x.css', './y.css'], './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('ignores a block comment inside a spread', () => {
      expect(readOf(`[.../* , './leak.css' */ SHARED, './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('ignores an apostrophe and a comma inside a line comment in a spread', () => {
      expect(readOf(`[...SHARED // don't , './leak.css'\n, './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('skips an interpolated template literal inside a spread', () => {
      // The `${...}` pushes a brace context, so the comma inside the template
      // does not end the element.
      expect(readOf("[...tag`${DIR}/a, b`, './own.css']")).toEqual({
        literals: ['./own.css'],
        complete: true,
      })
    })

    it('drops a spread in a `styleUrls` array too', () => {
      const src = `@Component({ styleUrls: [...SHARED, './a.css'] })\nclass Foo {}`
      expect(readStringLiterals(src, locateStyleUrlsFor(src, 'Foo')!)).toEqual({
        literals: ['./a.css'],
        complete: true,
      })
    })

    it('drops a spread in an inline `styles` array', () => {
      expect(readOf(`[...SHARED_INLINE, ':host{}']`)).toEqual({
        literals: [':host{}'],
        complete: true,
      })
    })

    it('reports an unterminated string inside a spread as incomplete', () => {
      // The quote never closes, so where the element ends is unknowable.
      // A real file cannot produce this range (the locator needs a balanced
      // `]`), but the scan must refuse to guess rather than run off.
      const src = `[...f('./leak.css)]`
      expect(readStringLiterals(src, [0, src.length - 1])).toEqual({
        literals: [],
        complete: false,
      })
    })

    it('reports an unclosed paren inside a spread as incomplete', () => {
      const src = `[...f(a]`
      expect(readStringLiterals(src, [0, src.length - 1])).toEqual({
        literals: [],
        complete: false,
      })
    })

    it('reports a spread ending in an escape as incomplete', () => {
      // The trailing escape consumes the array's own `]`, so the scan
      // overshoots the range: incomplete, not a guess.
      const src = String.raw`[...'a\]`
      expect(readStringLiterals(src, [0, src.length - 1])).toEqual({
        literals: [],
        complete: false,
      })
    })

    it('does not treat a leading decimal point as a spread', () => {
      expect(readOf(`[.5, './a.css']`).complete).toBe(false)
    })

    it('does not treat two dots as a spread', () => {
      expect(readOf(`[..X, './a.css']`).complete).toBe(false)
    })

    it('does not treat a member access as a spread', () => {
      expect(readOf(`[STYLES.a, './a.css']`).complete).toBe(false)
    })

    it('does not treat an optional chain as a spread', () => {
      expect(readOf(`[STYLES?.a, './a.css']`).complete).toBe(false)
    })

    it('still reports a bare identifier element as incomplete, unlike a spread', () => {
      // Measured: the Rust extractor DOES fold `S` from the const table, so
      // the literals gathered here are not the component's full style list.
      expect(readOf(`[S, './own.css']`)).toEqual({
        literals: ['./own.css'],
        complete: false,
      })
    })

    it('reads across a leading elision, which the compiler also drops', () => {
      expect(readOf(`[, './a.css']`)).toEqual({ literals: ['./a.css'], complete: true })
    })

    it('reads across an elision between two entries', () => {
      expect(readOf(`['./a.css', , './b.css']`)).toEqual({
        literals: ['./a.css', './b.css'],
        complete: true,
      })
    })

    it('reports an interpolated template literal as incomplete', () => {
      // The raw slice is `${DIR}/a.css`, not a real path. The Rust extractor
      // folds it; this scan cannot.
      expect(readOf('[`${DIR}/a.css`]').complete).toBe(false)
    })

    it('reports a bare interpolated template literal as incomplete', () => {
      expect(readOf('`${DIR}/a.css`').complete).toBe(false)
    })

    it('treats an escaped `${` in a template literal as ordinary text', () => {
      // `hasInterpolation` reads the odd backslash as an escape, so the
      // literal is complete; decoding then resolves `\$` to `$`, leaving the
      // `${…}` as the literal text it denotes. The two must agree.
      expect(readOf('[`\\${NOT_INTERPOLATED}.css`]')).toEqual({
        literals: ['${NOT_INTERPOLATED}.css'],
        complete: true,
      })
    })

    it('treats `${` inside a quoted string as ordinary text', () => {
      expect(readOf(`['\${DIR}/a.css']`)).toEqual({
        literals: ['${DIR}/a.css'],
        complete: true,
      })
    })

    it('stops at an unterminated literal and reports incomplete', () => {
      // The locator bounds the range, so the unterminated entry is dropped.
      const src = `@Component({ styles: ['./a.css', './b.css] })\nclass Foo {}`
      const range = locateStylesFieldFor(src, 'Foo')
      if (range) {
        const read = readStringLiterals(src, range)
        expect(read.literals).toEqual(['./a.css'])
        expect(read.complete).toBe(false)
      }
    })
  })

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

    // A comment may sit anywhere inside a style field declaration, and the
    // Rust extractor reads straight past it. Every placement below was
    // measured against `extractComponentUrls`, which reports `./x.css` for
    // each one. Reading any of them as absent would tell the caller the
    // class declares no styles, which strips the component's CSS.
    describe('comment placement within a style field declaration', () => {
      const urlsOf = (src: string) => {
        const field = locateStyleFieldsFor(src, 'Foo')!.urls
        expect(field.kind).toBe('literal')
        return readStringLiterals(src, (field as { range: [number, number] }).range).literals
      }
      const decorator = (field: string) =>
        `@Component({ template: '<p/>', ${field} })\nexport class Foo {}`

      it('reads a field with a block comment between the key and the colon', () => {
        expect(urlsOf(decorator(`styleUrls /* why */: ['./x.css']`))).toEqual(['./x.css'])
      })

      it('reads a field with a line comment between the key and the colon', () => {
        expect(urlsOf(decorator(`styleUrls // why\n: ['./x.css']`))).toEqual(['./x.css'])
      })

      it('reads a field with a comment between the colon and the value', () => {
        expect(urlsOf(decorator(`styleUrls: /* why */ ['./x.css']`))).toEqual(['./x.css'])
      })

      it('reads a field with a comment before the key', () => {
        expect(urlsOf(decorator(`/* why */ styleUrls: ['./x.css']`))).toEqual(['./x.css'])
      })

      it('reads a field with two comments between the key and the colon', () => {
        expect(urlsOf(decorator(`styleUrls /* a */ /* b */: ['./x.css']`))).toEqual(['./x.css'])
      })

      it('reads a quoted key with a comment before the colon', () => {
        expect(urlsOf(decorator(`'styleUrls' /* why */: ['./x.css']`))).toEqual(['./x.css'])
      })

      it('reads the singular `styleUrl` with a comment before the colon', () => {
        expect(urlsOf(decorator(`styleUrl /* why */: './x.css'`))).toEqual(['./x.css'])
      })

      it('reads the singular `styleUrl` with a comment after the colon', () => {
        expect(urlsOf(decorator(`styleUrl: /* why */ './x.css'`))).toEqual(['./x.css'])
      })

      it('reads an array whose first literal follows a comment', () => {
        expect(urlsOf(decorator(`styleUrls: [/* why */ './x.css']`))).toEqual(['./x.css'])
      })

      it('reads an array whose first literal follows a line comment', () => {
        expect(urlsOf(decorator(`styleUrls: [// why\n './x.css']`))).toEqual(['./x.css'])
      })

      it('reads an array with a comment before the separating comma', () => {
        expect(urlsOf(decorator(`styleUrls: ['./x.css' /* why */, './y.css']`))).toEqual([
          './x.css',
          './y.css',
        ])
      })

      it('reads an array with a comment after the separating comma', () => {
        expect(urlsOf(decorator(`styleUrls: ['./x.css', /* why */ './y.css']`))).toEqual([
          './x.css',
          './y.css',
        ])
      })

      it('reads an array with a comment after the last literal', () => {
        expect(urlsOf(decorator(`styleUrls: ['./x.css' /* why */]`))).toEqual(['./x.css'])
      })

      it('reads an inline `styles` field with a comment before the colon', () => {
        const src = decorator(`styles /* why */: ['.a{}']`)
        const field = locateStyleFieldsFor(src, 'Foo')!.inline
        expect(field.kind).toBe('literal')
        expect(
          readStringLiterals(src, (field as { range: [number, number] }).range).literals,
        ).toEqual(['.a{}'])
      })

      // An array holding only a comment is still a syntactically valid empty
      // array: a definite "no styles", not an unreadable value.
      it('reads an array holding only a comment as definitively empty', () => {
        const src = decorator(`styleUrls: [/* why */]`)
        const field = locateStyleFieldsFor(src, 'Foo')!.urls
        expect(field.kind).toBe('literal')
        expect(
          readStringLiterals(src, (field as { range: [number, number] }).range).literals,
        ).toEqual([])
      })

      // Comment *contents* must never be mistaken for structure.
      it('ignores a colon, comma and brackets inside the comment', () => {
        expect(urlsOf(decorator(`styleUrls /* a: b, c ] [ */: ['./x.css']`))).toEqual(['./x.css'])
      })

      it('ignores a decoy style field inside the comment', () => {
        expect(urlsOf(decorator(`styleUrls /* styleUrl: './fake.css' */: ['./x.css']`))).toEqual([
          './x.css',
        ])
      })

      it('ignores an apostrophe inside a comment between the key and the colon', () => {
        expect(urlsOf(decorator(`styleUrls /* don't */: ['./x.css']`))).toEqual(['./x.css'])
      })

      // A comment must not turn a shorthand back into "absent": the compiler
      // resolves a same-file constant behind the singular `styleUrl`
      // (verified: `./sh.css`), so this class must still reach the fallback.
      it('keeps a shorthand `styleUrl` unreadable when a comment precedes the closing brace', () => {
        const src = `const styleUrl = './sh.css';
@Component({ template: '<p/>', styleUrl /* why */ })
export class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
      })

      it('keeps a shorthand `styleUrl` unreadable when a comment precedes the comma', () => {
        const src = `const styleUrl = './sh.css';
@Component({ template: '<p/>', styleUrl /* why */, selector: 'a' })
export class Foo {}`
        expect(locateStyleFieldsFor(src, 'Foo')!.urls).toEqual({ kind: 'unreadable' })
      })
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

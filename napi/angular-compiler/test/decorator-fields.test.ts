import { describe, expect, it } from 'vitest'

import {
  emptyDelimitedRange,
  locateComponentDecorators,
  locateStylesInArgs,
  locateTemplateInArgs,
} from '../vite-plugin/utils/decorator-fields.js'

// The className-keyed wrappers that used to live in decorator-fields.ts went
// away with the text-scan HMR path — per-class resources now come from the
// Rust extractor. What survives is the metadata *strip* that decides HMR
// versus full reload, which reaches the same locators through
// `locateComponentDecorators`. These two mirror what the removed wrappers
// did, so the tests below still pin behaviour that ships.
const stylesFieldFor = (code: string, className: string): [number, number] | null => {
  const found = locateComponentDecorators(code).find((d) => d.className === className)
  return found ? locateStylesInArgs(code, found.argsRange) : null
}

const templateFieldFor = (code: string, className: string): [number, number] | null => {
  const found = locateComponentDecorators(code).find((d) => d.className === className)
  return found ? locateTemplateInArgs(code, found.argsRange) : null
}

/** The source text a located range covers, outer delimiters included. */
const textOf = (code: string, range: [number, number]): string => code.slice(range[0], range[1] + 1)

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
        `@Component({ selector: 'x', styles: ['.real{}'] })`,
        `// @Component({ styles: ['.old{}'] })`,
        `export class FooComponent {}`,
      ].join('\n')
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(out[0].className).toBe('FooComponent')
      expect(textOf(src, stylesFieldFor(src, 'FooComponent')!)).toBe(`['.real{}']`)
    })

    it('ignores a decorator inside a block comment that follows the real one', () => {
      const src = [
        `@Component({ selector: 'x', styles: ['.real{}'] })`,
        `/* @Component({ styles: ['.old{}'] }) */`,
        `export class FooComponent {}`,
      ].join('\n')
      expect(textOf(src, stylesFieldFor(src, 'FooComponent')!)).toBe(`['.real{}']`)
    })

    it('ignores a commented-out decorator that precedes the real one', () => {
      const src = [
        `// @Component({ styles: ['.old{}'] })`,
        `@Component({ selector: 'x', styles: ['.real{}'] })`,
        `export class FooComponent {}`,
      ].join('\n')
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(textOf(src, stylesFieldFor(src, 'FooComponent')!)).toBe(`['.real{}']`)
    })

    it('ignores a decorator written inside a string literal', () => {
      const src = [
        `const doc = 'see @Component({ styles: [".str{}"] }) for details';`,
        `@Component({ selector: 'x', styles: ['.real{}'] })`,
        `export class FooComponent {}`,
      ].join('\n')
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(textOf(src, stylesFieldFor(src, 'FooComponent')!)).toBe(`['.real{}']`)
    })

    it('pairs each class with its own decorator when a phantom sits between them', () => {
      const src = [
        `@Component({ selector: 'a', styles: ['.a{}'] })`,
        `// @Component({ styles: ['.fake{}'] })`,
        `export class AComponent {}`,
        `@Component({ selector: 'b', styles: ['.b{}'] })`,
        `export class BComponent {}`,
      ].join('\n')
      expect(locateComponentDecorators(src).map((d) => d.className)).toEqual([
        'AComponent',
        'BComponent',
      ])
      expect(textOf(src, stylesFieldFor(src, 'AComponent')!)).toBe(`['.a{}']`)
      expect(textOf(src, stylesFieldFor(src, 'BComponent')!)).toBe(`['.b{}']`)
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

  describe('locateStylesInArgs', () => {
    const multi = `
      @Component({ selector: 'a', styles: ['.first {}'] })
      export class FirstComponent {}
      @Component({ selector: 'b', styles: ['.second {}'] })
      export class SecondComponent {}
    `

    it('returns null when className matches no decorator', () => {
      expect(stylesFieldFor(multi, 'Nope')).toBeNull()
    })

    it('returns null when the named component has no styles field', () => {
      const src = `@Component({ template: '<p/>' })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('returns the FirstComponent styles range when asked for FirstComponent', () => {
      expect(textOf(multi, stylesFieldFor(multi, 'FirstComponent')!)).toBe(`['.first {}']`)
    })

    it('returns the SecondComponent styles range when asked for SecondComponent', () => {
      expect(textOf(multi, stylesFieldFor(multi, 'SecondComponent')!)).toBe(`['.second {}']`)
    })

    it('supports the bare-string styles form per component', () => {
      const src = `
        @Component({ styles: '.first {}' })
        export class FirstComponent {}
        @Component({ styles: '.second {}' })
        export class SecondComponent {}
      `
      expect(textOf(src, stylesFieldFor(src, 'SecondComponent')!)).toBe(`'.second {}'`)
    })

    // The next four guard against false-matches: a `styles:` key occurring
    // inside another field's string/template literal must not be picked up.
    it('ignores `styles:` text inside a template literal that precedes the real styles', () => {
      const src =
        "@Component({ template: `<pre>const cfg = { styles: ['fake'] }</pre>`, styles: ['real'] })\nexport class Foo {}"
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('returns null when the only `styles:` text in the args is inside a template literal', () => {
      const src = "@Component({ template: `<pre>{ styles: ['fake'] }</pre>` })\nexport class Bar {}"
      expect(stylesFieldFor(src, 'Bar')).toBeNull()
    })

    it("ignores `styles:` inside a `${...}` interpolation's nested object literal", () => {
      // `${ ... { styles: [...] } ... }` inside a template literal must not
      // be treated as a top-level @Component metadata property.
      const src =
        "@Component({ template: `${doThing({ styles: ['fake'] })}`, styles: ['real'] })\nexport class Baz {}"
      expect(textOf(src, stylesFieldFor(src, 'Baz')!)).toBe(`['real']`)
    })

    it('ignores `styles:` inside a nested non-metadata object literal', () => {
      // `metadata: { styles: ['nested'] }` is not the component's `styles`
      // field; only top-level properties of the @Component argument count.
      const src = `@Component({ host: { '[styles]': 'expr', styles: 'irrelevant' }, styles: ['real'] })\nexport class Qux {}`
      expect(textOf(src, stylesFieldFor(src, 'Qux')!)).toBe(`['real']`)
    })

    it('does not match a `styleUrls:` field as the inline `styles:` field', () => {
      const src = `@Component({ styleUrls: ['./a.css'] })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not match the singular `styleUrl:` field as the inline `styles:` field', () => {
      const src = `@Component({ styleUrl: './a.css' })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })
  })

  describe('locateTemplateInArgs', () => {
    const multi = `
      @Component({ selector: 'a', template: '<first/>' })
      export class FirstComponent {}
      @Component({ selector: 'b', template: '<second/>' })
      export class SecondComponent {}
    `

    it('returns null when className matches no decorator', () => {
      expect(templateFieldFor(multi, 'Nope')).toBeNull()
    })

    it('returns null when the named component has no template field', () => {
      const src = `@Component({ styles: [] })\nexport class Foo {}`
      expect(templateFieldFor(src, 'Foo')).toBeNull()
    })

    it('returns the FirstComponent template range when asked for FirstComponent', () => {
      expect(textOf(multi, templateFieldFor(multi, 'FirstComponent')!)).toBe(`'<first/>'`)
    })

    it('returns the SecondComponent template range when asked for SecondComponent', () => {
      expect(textOf(multi, templateFieldFor(multi, 'SecondComponent')!)).toBe(`'<second/>'`)
    })

    it("ignores `template:` text appearing inside another field's string literal", () => {
      // The `styles` array contains a string with literal `template:` text;
      // the real `template:` field comes after. The naive regex would match
      // the inner one first.
      const src = `@Component({ styles: ['/* template: "fake" */'], template: '<real/>' })\nexport class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<real/>'`)
    })

    it('does not match a `templateUrl:` field as `template:`', () => {
      const src = `@Component({ templateUrl: './foo.html' })\nexport class Foo {}`
      expect(templateFieldFor(src, 'Foo')).toBeNull()
    })

    it('finds the inline template when the decorator also has a templateUrl field', () => {
      const src = `@Component({ templateUrl: './real.html', template: '<p/>' })\nexport class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<p/>'`)
    })
  })

  // -----------------------------------------------------------------
  // Which property keys count as the field, and which must not. The
  // strip has to empty exactly the component's own `template:`/`styles:`
  // — emptying anything else, or missing the real one, changes the
  // stripped bytes and flips the HMR / full-reload decision.
  // -----------------------------------------------------------------
  describe('which keys count as the field', () => {
    it('does not locate a field whose value has no literal shape', () => {
      const src = `@Component({ styles: STYLES })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('reads a single-quoted key the same as the bare form', () => {
      const src = `@Component({ 'styles': ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('reads a double-quoted key the same as the bare form', () => {
      const src = `@Component({ "styles": ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('reads a quoted `template` key', () => {
      const src = `@Component({ 'template': '<p/>' })\nexport class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<p/>'`)
    })

    it('keeps the cross-match guards for quoted keys', () => {
      const urls = `@Component({ 'styleUrls': ['./a.css'] })\nexport class Foo {}`
      expect(stylesFieldFor(urls, 'Foo')).toBeNull()
      const tplUrl = `@Component({ 'templateUrl': './a.html' })\nexport class Foo {}`
      expect(templateFieldFor(tplUrl, 'Foo')).toBeNull()
    })

    it('reads the real field past a computed key', () => {
      // A computed key hides its name from this scan, but it is not the
      // field we are after, and the visible field is still exactly what it
      // says it is.
      const src = `@Component({ [K]: 1, styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('does not locate a computed key as a field', () => {
      const src = `@Component({ [K]: ['.x{}'] })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('reads the real field past a spread', () => {
      const src = `@Component({ ...BASE, styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('does not locate a shorthand style field, which has no value to empty', () => {
      const src = `@Component({ template: '<p/>', styles })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not let an unrelated shorthand degrade a readable field', () => {
      const src = `@Component({ selector, styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('reads the real field past a field-named identifier used as a value', () => {
      const src = `@Component({ selector: styles, styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('does not let an unrelated method degrade a readable field', () => {
      const src = `@Component({ foo() { return 1 }, styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    // A method or accessor named like the field is not the field: there is
    // no value literal to empty, and emptying its body would be wrong.
    it('does not locate a method named like the field', () => {
      const src = `@Component({ styles() { return ['.x{}'] } })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not locate a getter named like the field', () => {
      const src = `@Component({ get template() { return '<p/>' } })\nexport class Foo {}`
      expect(templateFieldFor(src, 'Foo')).toBeNull()
    })

    it('reads the real field past a setter named like it', () => {
      const src = `@Component({ set styles(v) {}, styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('does not read a field nested in a deeper object', () => {
      const src = `@Component({ data: { styles: ['.deep{}'] } })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('reads a field followed by a trailing comma', () => {
      const src = `@Component({ styles: ['.x{}'], })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })
  })

  // A value that merely STARTS with a literal does not denote it. Emptying
  // that leading piece would leave the rest of the expression in the
  // stripped source, so an edit to the expression would read as a
  // non-metadata change — or worse, an edit elsewhere would not.
  describe('a literal that is only the start of a larger expression', () => {
    const styles = (field: string) =>
      stylesFieldFor(`@Component({ selector: 'a', ${field} })\nexport class Foo {}`, 'Foo')
    const template = (field: string) =>
      templateFieldFor(`@Component({ selector: 'a', ${field} })\nexport class Foo {}`, 'Foo')

    it('rejects an inline `styles` string concatenated with an identifier', () => {
      expect(styles(`styles: '.a{}' + EXTRA`)).toBeNull()
    })

    it('rejects a `template` concatenated with another literal', () => {
      expect(template(`template: '<p/>' + '<q/>'`)).toBeNull()
    })

    it('rejects a `template` concatenated with an identifier', () => {
      expect(template(`template: '<p/>' + SUFFIX`)).toBeNull()
    })

    it('rejects a `styles` array concatenated with an identifier', () => {
      expect(styles(`styles: ['.a{}'] + EXTRA`)).toBeNull()
    })

    it('rejects a method call on an inline `styles` string literal', () => {
      expect(styles(`styles: '.a{}'.replace('a', 'b')`)).toBeNull()
    })

    it('rejects a method call on a `styles` array literal', () => {
      expect(styles(`styles: ['.a{}'].concat(MORE)`)).toBeNull()
    })

    it('rejects a method call on a `template` literal', () => {
      expect(template(`template: '<p/>'.replace('a', 'b')`)).toBeNull()
    })

    it('rejects a TypeScript `as` assertion after an inline `styles` string', () => {
      expect(styles(`styles: '.a{}' as string`)).toBeNull()
    })

    it('rejects a TypeScript `as const` assertion after a `styles` array', () => {
      expect(styles(`styles: ['.a{}'] as const`)).toBeNull()
    })

    it('rejects a TypeScript `as` assertion after a `template`', () => {
      expect(template(`template: '<p/>' as string`)).toBeNull()
    })

    it('rejects a non-null assertion after an inline `styles` string', () => {
      expect(styles(`styles: '.a{}'!`)).toBeNull()
    })

    it('rejects a `satisfies` clause after an inline `styles` string', () => {
      expect(styles(`styles: '.a{}' satisfies string`)).toBeNull()
    })

    it('rejects a trailing expression hidden behind a comment', () => {
      expect(styles(`styles: '.a{}' /* why */ + EXTRA`)).toBeNull()
    })

    // The other side of the rule: a value the property really does end at
    // stays readable, whether a comma, the object's brace or a comment
    // closes it out.
    it.each([
      [`styles: ['.a{}']`, `the object's closing brace`, `['.a{}']`],
      [`styles: ['.a{}'],`, 'a trailing comma', `['.a{}']`],
      [`styles: '.a{}' /* why */`, 'a block comment then the brace', `'.a{}'`],
      [`styles: '.a{}' // why\n`, 'a line comment then the brace', `'.a{}'`],
    ])('still reads %j, ended by %s', (field, _why, expected) => {
      const src = `@Component({ selector: 'a', ${field} })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(expected)
    })

    it.each([
      [`template: '<p/>'`, `the object's closing brace`],
      [`template: '<p/>',`, 'a trailing comma'],
    ])('still reads %j, ended by %s', (field) => {
      const src = `@Component({ selector: 'a', ${field} })\nexport class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<p/>'`)
    })
  })

  // Escaped identifier keys. `styles` IS `styles` to the TypeScript
  // parser, so the strip has to empty it like any other spelling of the
  // field. Decoding keeps the match exact where refusing would silently
  // leave a real `styles:` field in the stripped source.
  describe('escaped keys', () => {
    it('decodes a \\uHHHH escape in a bare key', () => {
      const src = `@Component({ style\\u0073: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('decodes a \\u{…} escape in a bare key', () => {
      const src = `@Component({ style\\u{73}: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('decodes an escape at the first character of a bare key', () => {
      const src = `@Component({ \\u0073tyles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('decodes an escaped `template` key', () => {
      const src = `@Component({ templat\\u0065: '<p/>' })\nexport class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<p/>'`)
    })

    it('decodes a quoted key carrying an escape', () => {
      // A quoted key is a string literal, so it decodes by string rules.
      const src = `@Component({ 'style\\u0073': ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
    })

    it('does not locate an escaped shorthand, which has no value to empty', () => {
      const src = `@Component({ template: '<p/>', style\\u0073 })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not match a decoded key that names something else', () => {
      const src = `@Component({ style\\u0073Extra: ['.x{}'] })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('keeps the cross-match guards for decoded keys', () => {
      const urls = `@Component({ style\\u0055rls: ['./a.css'] })\nexport class Foo {}`
      expect(stylesFieldFor(urls, 'Foo')).toBeNull()
      const tplUrl = `@Component({ templat\\u0065Url: './a.html' })\nexport class Foo {}`
      expect(templateFieldFor(tplUrl, 'Foo')).toBeNull()
    })

    it('does not match a lookalike built from a non-ASCII letter', () => {
      // Cyrillic е in place of `e` — a different identifier entirely.
      const src = `@Component({ stylеs: ['.x{}'] })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not match a key whose \\u escape is malformed', () => {
      const src = `@Component({ style\\u00ZZs: ['.x{}'] })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not match a key carrying a \\xHH escape — illegal in an identifier', () => {
      const src = `@Component({ style\\x73: ['.x{}'] })\nexport class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
    })

    it('does not let an escaped unrelated key degrade a readable field', () => {
      const src = `@Component({ sel\\u0065ctor: 'a', styles: ['.x{}'] })\nexport class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['.x{}']`)
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
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('does not get stuck on apostrophes inside a block comment', () => {
      const src = `@Component({
  /* It's important: don't use these */
  styles: ['real']
})
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('ignores `styles:` inside a line comment', () => {
      const src = `@Component({
  // styles: ['fake'],
  styles: ['real']
})
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('ignores `styles:` inside a block comment', () => {
      const src = `@Component({
  /* styles: ['fake'] */
  styles: ['real']
})
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('ignores `template:` inside a block comment', () => {
      const src = `@Component({
  /* template: '<fake/>' */
  template: '<real/>'
})
class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<real/>'`)
    })

    it('returns null when the only `styles:` is inside a comment', () => {
      const src = `@Component({
  // styles: ['fake']
  selector: 'app-foo'
})
class Foo {}`
      expect(stylesFieldFor(src, 'Foo')).toBeNull()
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
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
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
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'http://x'`)
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('does NOT treat `/*` inside a string as a block comment', () => {
      const src = `@Component({ template: '/* not a comment */', styles: ['real'] })
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    // A comment may sit anywhere inside a field declaration, and the strip
    // has to read straight past it — both to find the field at all, and to
    // put the range's closing delimiter in the right place.
    describe('comment placement within a field declaration', () => {
      const decorator = (field: string) =>
        `@Component({ selector: 'a', ${field} })\nexport class Foo {}`
      const stylesText = (field: string) => {
        const src = decorator(field)
        return textOf(src, stylesFieldFor(src, 'Foo')!)
      }
      const templateText = (field: string) => {
        const src = decorator(field)
        return textOf(src, templateFieldFor(src, 'Foo')!)
      }

      it('reads a field with a block comment between the key and the colon', () => {
        expect(stylesText(`styles /* why */: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('reads a field with a line comment between the key and the colon', () => {
        expect(stylesText(`styles // why\n: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('reads a field with a comment between the colon and the value', () => {
        expect(stylesText(`styles: /* why */ ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('reads a field with a comment before the key', () => {
        expect(stylesText(`/* why */ styles: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('reads a field with two comments between the key and the colon', () => {
        expect(stylesText(`styles /* a */ /* b */: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('reads a quoted key with a comment before the colon', () => {
        expect(stylesText(`'styles' /* why */: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('reads `template` with a comment before the colon', () => {
        expect(templateText(`template /* why */: '<p/>'`)).toBe(`'<p/>'`)
      })

      it('reads `template` with a comment after the colon', () => {
        expect(templateText(`template: /* why */ '<p/>'`)).toBe(`'<p/>'`)
      })

      // The range has to span the whole array, comments included — the strip
      // empties everything between the brackets.
      it('spans an array whose first entry follows a block comment', () => {
        expect(stylesText(`styles: [/* why */ '.x{}']`)).toBe(`[/* why */ '.x{}']`)
      })

      it('spans an array whose first entry follows a line comment', () => {
        expect(stylesText(`styles: [// why\n '.x{}']`)).toBe(`[// why\n '.x{}']`)
      })

      it('spans an array with a comment before the separating comma', () => {
        expect(stylesText(`styles: ['.x{}' /* why */, '.y{}']`)).toBe(`['.x{}' /* why */, '.y{}']`)
      })

      it('spans an array with a comment after the last entry', () => {
        expect(stylesText(`styles: ['.x{}' /* why */]`)).toBe(`['.x{}' /* why */]`)
      })

      it('does not let a `]` inside a comment close the array early', () => {
        expect(stylesText(`styles: ['.x{}' /* ] */]`)).toBe(`['.x{}' /* ] */]`)
      })

      // An array holding only a comment is still a syntactically valid empty
      // array, and emptying it is still the right answer.
      it('spans an array holding only a comment, which strips to `[]`', () => {
        const src = decorator(`styles: [/* why */]`)
        const range = stylesFieldFor(src, 'Foo')!
        expect(textOf(src, range)).toBe(`[/* why */]`)
        expect(emptyDelimitedRange(src, range)).toContain(`styles: []`)
      })

      // Comment *contents* must never be mistaken for structure.
      it('ignores a colon, comma and brackets inside the comment', () => {
        expect(stylesText(`styles /* a: b, c ] [ */: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('ignores a decoy field inside the comment', () => {
        expect(stylesText(`styles /* styles: './fake.css' */: ['.x{}']`)).toBe(`['.x{}']`)
      })

      it('ignores an apostrophe inside a comment between the key and the colon', () => {
        expect(stylesText(`styles /* don't */: ['.x{}']`)).toBe(`['.x{}']`)
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
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<real/>'`)
    })

    it('ignores `@Component(...)` text inside a string literal preceding the real decorator', () => {
      const src = `const docs = "use @Component({ template: 'fake' }) to declare"
@Component({ template: '<real/>' })
class Foo {}`
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<real/>'`)
    })

    it('ignores `@Component(...)` text inside a backtick template preceding the real decorator', () => {
      const src =
        "`Use @Component({ template: 'fake' })`\n@Component({ template: '<real/>' })\nclass Foo {}"
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<real/>'`)
    })

    it('does not get confused by a `template:` literal that mentions the word "styles:"', () => {
      const src = `@Component({ template: 'styles: ["fake"]', styles: ['real'] })
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('handles unbalanced braces or brackets inside template string content', () => {
      const src = `@Component({ template: 'has { and ] literally', styles: ['real'] })
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('treats CRLF line endings the same as LF', () => {
      const src = `@Component({\r\n  // a comment with an apostrophe: I'm here\r\n  styles: ['real']\r\n})\r\nclass Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('handles `...spread` followed by a real `styles:` field', () => {
      const src = `const base = { selector: 'app' }
@Component({ ...base, styles: ['real'] })
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
    })

    it('handles a selector value that contains parens', () => {
      const src = `@Component({ selector: 'foo(bar)', styles: ['real'] })
class Foo {}`
      expect(textOf(src, stylesFieldFor(src, 'Foo')!)).toBe(`['real']`)
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
      expect(textOf(src, templateFieldFor(src, 'First')!)).toBe(`'<a/>'`)
      expect(textOf(src, templateFieldFor(src, 'Second')!)).toBe(`'<b/>'`)
    })

    it('handles literal `$` followed by `${...}` interpolation in a template literal', () => {
      const src = '@Component({ template: `cost $5 or $${price}` })\nclass Foo {}'
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe('`cost $5 or $${price}`')
    })

    it('coexists with other class-level decorators like @SignalComponent', () => {
      const src = `@SignalComponent({ template: 'sig' })
@Component({ template: '<real/>' })
class Foo {}`
      // Only @Component is recognized; @SignalComponent is ignored entirely.
      const out = locateComponentDecorators(src)
      expect(out).toHaveLength(1)
      expect(textOf(src, templateFieldFor(src, 'Foo')!)).toBe(`'<real/>'`)
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

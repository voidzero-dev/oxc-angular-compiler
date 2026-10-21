import { describe, expect, it } from 'vitest'

import { compareFullFileSemantically } from './compare.js'

const component = (styles: string, extra = '') => `
import * as i0 from "@angular/core";
export class MyComponent {
  ${extra}
  static ɵcmp = /*@__PURE__*/ i0.ɵɵdefineComponent({ type: MyComponent, selectors: [["my-cmp"]], decls: 0, vars: 0, template: function MyComponent_Template(rf, ctx) {}, styles: [${styles}], encapsulation: 0 });
}
`

describe('compareFullFileSemantically', () => {
  it('ignores formatting, quotes, trailing commas and plain template literals', async () => {
    const tsCode = component('"a[_ngcontent-%COMP%] {color: red;}"')
    const oxcCode = component('\n  `a[_ngcontent-%COMP%] {color: red;}`,\n')
    expect((await compareFullFileSemantically(oxcCode, tsCode)).match).toBe(true)
  })

  it('reports whitespace differences inside string literals', async () => {
    const result = await compareFullFileSemantically(
      component('"one[_ngcontent-%COMP%] > two {}"'),
      component('"one[_ngcontent-%COMP%]    > two {}"'),
    )
    expect(result.match).toBe(false)
    expect(result.staticFieldDiffs?.map((d) => `${d.className}.${d.fieldName}`)).toEqual([
      'MyComponent.ɵcmp',
    ])
  })

  it('does not rewrite ", }" or " }" inside strings', async () => {
    const result = await compareFullFileSemantically(component('"a {}"'), component('"a { }"'))
    expect(result.match).toBe(false)
  })

  it('compares exported class members other than the static ɵ fields', async () => {
    const result = await compareFullFileSemantically(
      component('""', 'open = signal(false);'),
      component('""', 'open = signal(false, ...(ngDevMode ? [{ debugName: "open" }] : []));'),
    )
    expect(result.match).toBe(false)
    expect(result.classDiffs?.map((d) => d.className)).toEqual(['MyComponent'])
    expect(result.staticFieldDiffs).toBeUndefined()
  })

  it('compares exported functions and hoisted constants by value', async () => {
    const tsCode = `const _c0 = ["*"];\nexport function f() { return _c0; }\n`
    expect((await compareFullFileSemantically(tsCode, tsCode.replace('"*"', "'*'"))).match).toBe(
      true,
    )
    const result = await compareFullFileSemantically(tsCode.replace('"*"', '"a"'), tsCode)
    expect(result.match).toBe(false)
    expect(result.functionComparison?.functionDiffs.map((d) => d.name)).toEqual(['_c0'])
  })

  it('pairs hoisted constants emitted in a different order', async () => {
    const tsCode = `const _c0 = ["a"];\nconst _c1 = ["b"];\nexport function f() { return [_c0, _c1]; }\n`
    const oxcCode = `const _c0 = ["b"];\nconst _c1 = ["a"];\nexport function f() { return [_c1, _c0]; }\n`
    expect((await compareFullFileSemantically(oxcCode, tsCode)).match).toBe(true)
  })
})

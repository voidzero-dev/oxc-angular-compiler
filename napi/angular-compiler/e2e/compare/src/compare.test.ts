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

  it('reports an export that points at another binding', async () => {
    const result = await compareFullFileSemantically(
      'class A {}\nclass B {}\nexport { A as X };\n',
      'class A {}\nclass B {}\nexport { B as X };\n',
    )
    expect(result.match).toBe(false)
    expect(result.exportDiffs).toEqual([
      { type: 'different', exportName: 'X', expected: 'B', actual: 'A' },
    ])
  })

  it('compares classes that contain BigInt literals', async () => {
    const result = await compareFullFileSemantically(
      'export class A { x = 10n; y = 1; }\n',
      'export class A { x = 10n; y = 2; }\n',
    )
    expect(result.parseErrors).toBeUndefined()
    expect(result.classDiffs?.map((d) => d.className)).toEqual(['A'])
    expect(
      (
        await compareFullFileSemantically(
          'export class A { x = 1n; }',
          'export class A { x = 2n; }',
        )
      ).classDiffs,
    ).toHaveLength(1)
  })

  describe('top-level statements', () => {
    const metadata = (cls: string) =>
      `(() => { (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(${cls}, [], null, null); })();`
    const debugInfo = (cls: string, filePath: string, line = 3) =>
      `(() => { (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassDebugInfo(${cls}, { className: "${cls}", filePath: "${filePath}", lineNumber: ${line} }); })();`
    const file = (...statements: string[]) =>
      `import * as i0 from "@angular/core";\nexport class A {}\n${statements.join('\n')}\n`
    const labels = async (oxc: string, ts: string) =>
      (await compareFullFileSemantically(oxc, ts)).statementDiffs?.map(
        (d) => `${d.type} ${d.label}`,
      )

    it('reports a missing setNgModuleScope call', async () => {
      const scope =
        '(function () { (typeof ngJitMode === "undefined" || ngJitMode) && i0.ɵɵsetNgModuleScope(A, { imports: [] }); })();'
      expect(await labels(file(metadata('A')), file(metadata('A'), scope))).toEqual([
        'missing ɵɵsetNgModuleScope(A)',
      ])
    })

    it('tells a debug info file path difference apart from other differences', async () => {
      const ts = file(metadata('A'), debugInfo('A', 'a.ts'))
      expect(await labels(file(metadata('A'), debugInfo('A', '/x/a.ts')), ts)).toEqual([
        'different ɵsetClassDebugInfo(A) filePath',
      ])
      expect(await labels(file(metadata('A'), debugInfo('A', '/x/a.ts', 4)), ts)).toEqual([
        'different ɵsetClassDebugInfo(A)',
      ])
      expect((await compareFullFileSemantically(ts, ts.replace('"a.ts"', "'a.ts'"))).match).toBe(
        true,
      )
    })

    it('reports a statement emitted ahead of one Angular emits first', async () => {
      expect(
        await labels(
          file(debugInfo('A', 'a.ts'), metadata('A')),
          file(metadata('A'), debugInfo('A', 'a.ts')),
        ),
      ).toEqual(['order order ɵsetClassDebugInfo(A)'])
    })
  })
})

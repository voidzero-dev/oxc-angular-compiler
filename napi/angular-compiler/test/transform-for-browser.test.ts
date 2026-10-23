import { describe, it, expect } from 'vitest'

import { transformForBrowser, transformForBrowserSync } from '../index.js'

describe('transformForBrowser', () => {
  it('lowers async functions, async generators and for await to generators for zone.js', async () => {
    const result = await transformForBrowser(
      `export async function load(urls) { for await (const r of urls) await r; }
export async function* pages() { yield 1; }`,
      'load.mjs',
    )
    expect(result.errors).toEqual([])
    expect(result.code).not.toMatch(/async function|for await/)
    expect(result.code).toContain('@oxc-project/runtime/helpers/asyncToGenerator')
  })

  it("keeps async native with lowerAsync: false, and leaves JavaScript's class fields alone", () => {
    const result = transformForBrowserSync(
      `export class A { b = 1; static s = 2; async f() { await 1; } }`,
      'a.mjs',
      { lowerAsync: false },
    )
    expect(result.code).toContain('async f()')
    expect(result.code).toMatch(/b = 1;[\s\S]*static s = 2/)
  })

  it('strips TypeScript with useDefineForClassFields: false: fields assigned after parameter properties', () => {
    const { code, errors } = transformForBrowserSync(
      `export class A { b = this.a.x; static s = 1; c: number; constructor(private a: { x: number }) {} }`,
      'a.ts',
    )
    expect(errors).toEqual([])
    expect(code.indexOf('this.a = a')).toBeLessThan(code.indexOf('this.b = this.a.x'))
    expect(code).toContain('A.s = 1')
    expect(code).not.toContain('this.c')
  })

  it('applies legacy decorators to TypeScript', () => {
    const { code, errors } = transformForBrowserSync(
      `function d(t: any) {}\n@d export class A {}`,
      'a.ts',
    )
    expect(errors).toEqual([])
    expect(code).not.toContain('@d')
    expect(code).toContain('_decorate')
  })

  it('reports syntax errors', () => {
    expect(transformForBrowserSync('let = ;', 'broken.ts').errors.length).toBeGreaterThan(0)
  })
})

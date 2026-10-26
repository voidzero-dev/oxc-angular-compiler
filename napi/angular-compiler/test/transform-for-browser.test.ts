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
    expect(code).toMatch(/static \{\s*this\.s = 1;\s*\}/)
    expect(code).not.toContain('this.c')
  })

  it('keeps private members native and static blocks in source order, as tsc does', () => {
    const { code, errors } = transformForBrowserSync(
      `export class A { static s = 1; static { log(A.s) } #x = 1; #m() { return this.#x } }`,
      'a.ts',
    )
    expect(errors).toEqual([])
    expect(code).toContain('#x = 1;')
    expect(code).toContain('#m() {')
    expect(code).not.toContain('@oxc-project/runtime')
    expect(code.indexOf('this.s = 1')).toBeLessThan(code.indexOf('log(A.s)'))
  })

  it('transforms enums', () => {
    const { code, errors } = transformForBrowserSync(`export enum E { A, B }`, 'e.ts')
    expect(errors).toEqual([])
    expect(code).toContain('E[E["B"] = 1] = "B"')
  })

  it('lowers await using before async', () => {
    const { code, errors } = transformForBrowserSync(
      `export async function f(r) { await using x = r; return 1; }`,
      'f.mjs',
    )
    expect(errors).toEqual([])
    expect(code).not.toMatch(/using x|async function/)
  })

  it('returns a source map only when requested', () => {
    const code = `export async function f(a: number) { return await a; }`
    expect(transformForBrowserSync(code, 'f.ts').map ?? null).toBeNull()
    const map = JSON.parse(transformForBrowserSync(code, 'f.ts', { sourcemap: true }).map!)
    expect(map.sources).toEqual(['f.ts'])
    expect(map.mappings).not.toBe('')
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

  it('reports syntax errors with their position', () => {
    const { code, errors } = transformForBrowserSync('const a = 1;\nlet = ;', 'broken.ts')
    expect(code).toBe('')
    expect(errors.length).toBeGreaterThan(0)
    expect(errors[0].labels[0].start).toBeGreaterThan(12)
    expect(errors[0].codeframe).toContain('let = ;')
  })
})

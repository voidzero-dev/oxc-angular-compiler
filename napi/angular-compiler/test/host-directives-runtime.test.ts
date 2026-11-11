/**
 * `hostDirectives` at runtime (#702).
 *
 * - A partial declaration linked by either linker binds the host
 *   directive's own fields to the aliases written in `inputs` / `outputs`,
 *   and a host directive declared after the class (`forwardRef`) is read
 *   once the classes are defined.
 * - A full compile reaches an imported host directive by the module's export
 *   name, also through an alias, a namespace import or a string export name.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { linkAngularPackageSync, transformAngularFileSync } from '../index.js'

const TMP = join(fileURLToPath(new URL('.', import.meta.url)), '.host-directives-runtime-tmp')
let nextCase = 0

beforeAll(() => {
  mkdirSync(TMP, { recursive: true })
})

afterAll(() => {
  rmSync(TMP, { recursive: true, force: true })
})

/** Angular's linker, as the Angular CLI runs it on libraries. */
async function angularLink(code: string, filename: string): Promise<string> {
  const require = createRequire(import.meta.url)
  const babel = createRequire(require.resolve('@angular/compiler-cli/package.json'))('@babel/core')
  const { createEs2015LinkerPlugin } = await import('@angular/compiler-cli/linker/babel')
  const plugin = createEs2015LinkerPlugin({
    linkerJitMode: false,
    fileSystem: {
      resolve,
      exists: existsSync,
      dirname,
      relative,
      readFile: (file: string) => readFileSync(file, 'utf8'),
    } as any,
    logger: { level: 1, debug() {}, info() {}, warn() {}, error() {} },
  })
  return babel.transformSync(code, {
    filename,
    babelrc: false,
    configFile: false,
    plugins: [plugin],
  }).code
}

const LINKERS = {
  angular: angularLink,
  oxc: async (code: string, filename: string) => linkAngularPackageSync(code, filename).code,
}

/**
 * Writes `files` (`.mts` sources, importing each other as `./name.mjs`):
 * `lib` ones partially compiled and linked with `linker`, the others fully
 * compiled. Returns the directory.
 */
async function build(files: Record<string, string>, linker: keyof typeof LINKERS): Promise<string> {
  const dir = join(TMP, `case-${nextCase++}`)
  mkdirSync(dir, { recursive: true })
  for (const [name, source] of Object.entries(files)) {
    const file = join(dir, `${name}.mts`)
    const partial = name.startsWith('lib')
    const result = transformAngularFileSync(source, file, {
      compilationMode: partial ? 'partial' : 'full',
    })
    expect(result.errors).toEqual([])
    const out = join(dir, `${name}.mjs`)
    writeFileSync(out, partial ? await LINKERS[linker](result.code, out) : result.code)
  }
  return dir
}

/**
 * Loads `name` in plain Node and prints, for each exported class listed, its
 * host directives as `[directive name, inputs, outputs]`, resolving the
 * `forwardRef` list.
 */
function hostDirectives(dir: string, name: string, classes: string[]): unknown {
  const script = `
    globalThis.ngJitMode = false;
    globalThis.ngDevMode = false;
    const { ɵgetComponentDef: cmp, ɵgetDirectiveDef: dir } = await import('@angular/core');
    const m = await import(${JSON.stringify(pathToFileURL(join(dir, `${name}.mjs`)).href)});
    // A mapping is an object once the runtime has read it, and still the
    // flat [field, alias, ...] array inside a forwardRef list.
    const map = (v) => Array.isArray(v)
      ? Object.fromEntries(v.flatMap((x, i) => (i % 2 ? [] : [[x, v[i + 1]]])))
      : (v ?? {});
    const out = {};
    for (const name of ${JSON.stringify(classes)}) {
      const def = cmp(m[name]) || dir(m[name]);
      out[name] = def.hostDirectives.map((h) => {
        const entries = typeof h === 'function' ? h() : [h];
        return entries.map((e) => typeof e === 'function'
          ? [e.name, {}, {}]
          : [e.directive.name, map(e.inputs), map(e.outputs)]);
      }).flat();
    }
    console.log(JSON.stringify(out));
  `
  return JSON.parse(
    execFileSync(process.execPath, ['--input-type=module', '-e', script], {
      cwd: fileURLToPath(new URL('..', import.meta.url)),
      encoding: 'utf8',
    }),
  )
}

describe.each(Object.keys(LINKERS) as (keyof typeof LINKERS)[])('linked by %s', (linker) => {
  it('binds the host directive fields to their aliases', async () => {
    const dir = await build(
      {
        lib: `
          import { Component, Directive, EventEmitter, Input, Output } from '@angular/core';
          @Directive({ selector: '[l]' })
          export class L { @Input() color = ''; @Output() changed = new EventEmitter(); }
          @Component({ selector: 'c', template: '',
            hostDirectives: [{ directive: L, inputs: ['color: c'], outputs: ['changed: ch'] }] })
          export class Cmp {}
          @Directive({ selector: '[d]', hostDirectives: [{ directive: L, inputs: ['color: c'] }] })
          export class Dir {}
        `,
      },
      linker,
    )
    expect(hostDirectives(dir, 'lib', ['Cmp', 'Dir'])).toEqual({
      Cmp: [['L', { color: 'c' }, { changed: 'ch' }]],
      Dir: [['L', { color: 'c' }, {}]],
    })
  })

  it('reads a host directive declared after the class', async () => {
    const dir = await build(
      {
        lib: `
          import { Component, Directive, forwardRef, Input } from '@angular/core';
          @Component({ selector: 'c', template: '', hostDirectives: [forwardRef(() => L)] })
          export class Cmp {}
          @Directive({ selector: '[d]',
            hostDirectives: [{ directive: forwardRef(() => L), inputs: ['color: c'] }] })
          export class Dir {}
          @Directive({ selector: '[l]' }) export class L { @Input() color = ''; }
        `,
      },
      linker,
    )
    expect(hostDirectives(dir, 'lib', ['Cmp', 'Dir'])).toEqual({
      Cmp: [['L', {}, {}]],
      Dir: [['L', { color: 'c' }, {}]],
    })
  })
})

describe('full compile', () => {
  it('reaches an imported host directive by its export name', async () => {
    const dir = await build(
      {
        dep: `
          import { Directive } from '@angular/core';
          @Directive({ selector: '[h]' }) export class H {}
          @Directive({ selector: '[s]' }) class SDirImpl {}
          export { SDirImpl as "s-dir" };
        `,
        main: `
          import { Component, Directive } from '@angular/core';
          import { H as P, "s-dir" as SDir } from './dep.mjs';
          import * as dep from './dep.mjs';
          @Component({ selector: 'c', template: '', hostDirectives: [P, dep.H, SDir, dep["s-dir"]] })
          export class Cmp {}
          @Directive({ selector: '[d]', hostDirectives: [P, dep.H, SDir, dep["s-dir"]] })
          export class Dir {}
        `,
      },
      'oxc',
    )
    const expected = [
      ['H', {}, {}],
      ['H', {}, {}],
      ['SDirImpl', {}, {}],
      ['SDirImpl', {}, {}],
    ]
    expect(hostDirectives(dir, 'main', ['Cmp', 'Dir'])).toEqual({ Cmp: expected, Dir: expected })
  })
})

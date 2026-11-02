/**
 * The scope of a linked NgModule, at runtime.
 *
 * Oxc's linker keeps an NgModule's `declarations`, `imports` and `exports`,
 * which a component compiled by Oxc reads at runtime (Angular's linker leaves
 * them out, as ngtsc lists what a component uses itself). They are read
 * lazily, so a module and a component file that import each other load in
 * either order, as they do when Angular's linker links them.
 */
import { execFileSync } from 'node:child_process'
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { linkAngularPackageSync, transformAngularFileSync } from '../index.js'

const TMP = join(fileURLToPath(new URL('.', import.meta.url)), '.linker-ng-module-scope-tmp')
let nextCase = 0
let previous: { ngJitMode: unknown; ngDevMode: unknown }

beforeAll(() => {
  mkdirSync(TMP, { recursive: true })
  const g = globalThis as any
  previous = { ngJitMode: g.ngJitMode, ngDevMode: g.ngDevMode }
  g.ngJitMode = false
  // In dev mode `ɵsetClassMetadata` reads the decorator's `declarations`
  // eagerly, with either linker.
  g.ngDevMode = false
})

afterAll(() => {
  Object.assign(globalThis, previous)
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
 * compiled by Oxc. Returns the directory.
 */
async function build(files: Record<string, string>, linker: keyof typeof LINKERS): Promise<string> {
  const dir = join(TMP, `case-${nextCase++}`)
  mkdirSync(dir, { recursive: true })
  for (const [name, source] of Object.entries(files)) {
    const file = join(dir, `${name}.mts`)
    writeFileSync(file, source)
    const partial = name.startsWith('lib')
    const result = transformAngularFileSync(source, file, {
      compilationMode: partial ? 'partial' : 'full',
      resolveImportedValues: partial,
    })
    expect(result.errors).toEqual([])
    const out = join(dir, `${name}.mjs`)
    writeFileSync(out, partial ? await LINKERS[linker](result.code, out) : result.code)
  }
  return dir
}

const load = (dir: string, name: string) =>
  import(/* @vite-ignore */ pathToFileURL(join(dir, `${name}.mjs`)).href)

/**
 * Loads `first`, then runs `check` (a module body), in plain Node: Vitest's
 * module runner reads an import that isn't initialized yet as `undefined`
 * instead of throwing. Returns what `check` prints.
 */
function runInNode(dir: string, first: string, check = ''): string {
  const script = `
    globalThis.ngJitMode = false;
    globalThis.ngDevMode = false;
    await import(${JSON.stringify(pathToFileURL(join(dir, `${first}.mjs`)).href)});
    ${check}
  `
  return execFileSync(process.execPath, ['--input-type=module', '-e', script], {
    cwd: dir,
    encoding: 'utf8',
  })
}

/**
 * A module and a component file that import each other. The module exports
 * no class of the cycle: `ɵinj.imports` lists `exports` as written, like
 * ngtsc's local compilation.
 */
const CYCLE = {
  libComp: `
    import { Component, inject } from '@angular/core';
    import { TITLE } from './libMod.mjs';
    @Component({ selector: 'app', template: '<i d>{{ t }}</i>', standalone: false })
    export class App { t = inject(TITLE); }
  `,
  libMod: `
    import { Directive, InjectionToken, NgModule } from '@angular/core';
    import { App } from './libComp.mjs';
    export const TITLE = new InjectionToken('title', { factory: () => 'T' });
    @Directive({ selector: '[d]', standalone: false }) export class D {}
    @NgModule({ declarations: [App, D], exports: [D] }) export class AModule {}
  `,
}

describe.each(Object.keys(LINKERS) as (keyof typeof LINKERS)[])('linked by %s', (linker) => {
  it('loads a module and a component file that import each other in either order', async () => {
    for (const first of ['libComp', 'libMod']) {
      const dir = await build(CYCLE, linker)
      const check = `const { AModule } = await import('./libMod.mjs'); console.log(typeof AModule.ɵmod);`
      expect(runInNode(dir, first, check).trim()).toBe('object')
    }
  })
})

describe('linked by oxc', () => {
  it('keeps the scope of a module, read once the files have loaded', async () => {
    const dir = await build(CYCLE, 'oxc')
    const check = `
      const { App } = await import('./libComp.mjs');
      const { AModule, D } = await import('./libMod.mjs');
      const { declarations, exports } = AModule.ɵmod;
      console.log(JSON.stringify([declarations(), exports()].map((list) => list.map((c) => c.name))));
      console.log(declarations()[0] === App && exports()[0] === D);
    `
    expect(runInNode(dir, 'libComp', check).trim().split('\n')).toEqual([
      '[["App","D"],["D"]]',
      'true',
    ])
  })

  it('gives a component compiled by Oxc what an imported library NgModule exports', async () => {
    const dir = await build(
      {
        lib: `
          import { Directive, NgModule, Pipe } from '@angular/core';
          @Directive({ selector: '[d]', standalone: false }) export class D {}
          @Pipe({ name: 'up', standalone: false }) export class Up { transform(v) { return v; } }
          @NgModule({ declarations: [D, Up], exports: [D, Up] }) export class LibModule {}
          @NgModule({ imports: [LibModule], exports: [LibModule] }) export class ReModule {}
        `,
        main: `
          import { Component } from '@angular/core';
          import { ReModule } from './lib.mjs';
          @Component({ selector: 'app', template: '<i d>{{ "x" | up }}</i>', imports: [ReModule] })
          export class App {}
        `,
      },
      'oxc',
    )
    const { App } = await load(dir, 'main')
    const { D, Up } = await load(dir, 'lib')
    const dependencies = App.ɵcmp.dependencies
    const resolved = typeof dependencies === 'function' ? dependencies() : dependencies
    expect(resolved).toEqual(expect.arrayContaining([D, Up]))
  })
})

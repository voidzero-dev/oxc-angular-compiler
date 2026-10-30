/**
 * NgModule scopes of partially compiled libraries (#655), at runtime.
 *
 * A partial declaration of a component an NgModule declares carries no
 * `dependencies`, and the Angular CLI's linker emits library NgModules without
 * their scope, so the runtime can't find the component's directives and
 * pipes. The module's file sets them with `ɵɵsetComponentScope`, and a
 * standalone component lists what an imported NgModule exports. Each case is
 * linked with Angular's linker and with Oxc's, and run with the real
 * `@angular/core`.
 */
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { linkAngularPackageSync, transformAngularFileSync } from '../index.js'

const TMP = join(fileURLToPath(new URL('.', import.meta.url)), '.partial-ng-module-scope-tmp')
let nextCase = 0
let previousJitMode: unknown

beforeAll(() => {
  mkdirSync(TMP, { recursive: true })
  previousJitMode = (globalThis as any).ngJitMode
  ;(globalThis as any).ngJitMode = false
})

afterAll(() => {
  ;(globalThis as any).ngJitMode = previousJitMode
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
 * Partially compiles `files` (`.mts` sources, importing each other as
 * `./name.mjs`), links each with `linker` and loads `main`.
 */
async function load(
  files: Record<string, string>,
  linker: keyof typeof LINKERS,
): Promise<Record<string, any>> {
  const dir = join(TMP, `case-${nextCase++}`)
  mkdirSync(dir, { recursive: true })
  for (const [name, source] of Object.entries(files)) {
    writeFileSync(join(dir, `${name}.mts`), source)
  }
  for (const [name, source] of Object.entries(files)) {
    const file = join(dir, `${name}.mts`)
    const result = transformAngularFileSync(source, file, {
      compilationMode: 'partial',
      resolveImportedValues: true,
    })
    expect(result.errors).toEqual([])
    // Linked as the JavaScript a library ships: Angular's linker types the
    // code it writes into a `.ts` file.
    const linked = await LINKERS[linker](result.code, join(dir, `${name}.mjs`))
    writeFileSync(join(dir, `${name}.mjs`), linked)
  }
  return import(/* @vite-ignore */ pathToFileURL(join(dir, 'main.mjs')).href)
}

/** The class names of the directives and of the pipes `cmp`'s template can use. */
function scope(cmp: any): { directives: string[]; pipes: string[] } {
  const names = (defs: any) =>
    (typeof defs === 'function' ? defs() : (defs ?? [])).map((def: any) => def.type.name)
  return { directives: names(cmp.ɵcmp.directiveDefs), pipes: names(cmp.ɵcmp.pipeDefs) }
}

const DEP = `
  import { Directive, NgModule, Pipe } from '@angular/core';
  @Directive({ selector: '[d]', standalone: false }) export class D {}
  @Pipe({ name: 'p', standalone: false }) export class P { transform(v) { return v; } }
  @NgModule({ declarations: [D, P], exports: [D, P] }) export class DepModule {}
`

describe.each(Object.keys(LINKERS) as (keyof typeof LINKERS)[])('linked by %s', (linker) => {
  it('gives a declared component the scope of its NgModule', async () => {
    const m = await load(
      {
        dep: DEP,
        main: `
          import { Component, NgModule } from '@angular/core';
          import { DepModule } from './dep.mjs';
          @Component({ selector: 'app', template: '<i d>{{ 1 | p }}</i>', standalone: false })
          export class App {}
          @NgModule({ declarations: [App], imports: [DepModule] }) export class AppModule {}
        `,
      },
      linker,
    )
    expect(scope(m.App)).toEqual({ directives: ['D', 'App'], pipes: ['P'] })
  })

  it('gives a standalone component the exports of an imported NgModule', async () => {
    const m = await load(
      {
        dep: DEP,
        main: `
          import { Component } from '@angular/core';
          import { DepModule } from './dep.mjs';
          @Component({ selector: 'app', template: '<i d>{{ 1 | p }}</i>', imports: [DepModule] })
          export class App {}
        `,
      },
      linker,
    )
    expect(scope(m.App)).toEqual({ directives: ['D'], pipes: ['P'] })
  })

  it('sets the scope once the file has declared the component', async () => {
    const m = await load(
      {
        dep: DEP,
        main: `
          import { Component, NgModule, forwardRef } from '@angular/core';
          import { DepModule } from './dep.mjs';
          @NgModule({ declarations: [forwardRef(() => App)], imports: [DepModule] })
          export class AppModule {}
          @Component({ selector: 'app', template: '<i d></i>', standalone: false })
          export class App {}
        `,
      },
      linker,
    )
    expect(scope(m.App)).toEqual({ directives: ['D', 'App'], pipes: ['P'] })
  })

  it('lists a class an imported NgModule also exports once', async () => {
    const m = await load(
      {
        dep: `
          import { Directive, NgModule } from '@angular/core';
          @Directive({ selector: '[s]' }) export class S {}
          @NgModule({ imports: [S], exports: [S] }) export class SModule {}
        `,
        main: `
          import { Component } from '@angular/core';
          import { S, SModule } from './dep.mjs';
          @Component({ selector: 'app', template: '<i s></i>', imports: [SModule, S] })
          export class App {}
        `,
      },
      linker,
    )
    expect(scope(m.App)).toEqual({ directives: ['S'], pipes: [] })
  })

  it('reads an exported class declared after the component when it renders', async () => {
    const m = await load(
      {
        main: `
          import { Component, Directive, NgModule, forwardRef } from '@angular/core';
          @NgModule({ imports: [forwardRef(() => D)], exports: [forwardRef(() => D)] })
          export class M {}
          @Component({ selector: 'app', template: '<i d></i>', imports: [M] })
          export class App {}
          @Directive({ selector: '[d]' }) export class D {}
        `,
      },
      linker,
    )
    expect(scope(m.App)).toEqual({ directives: ['D'], pipes: [] })
  })

  it('links templates with block syntax', async () => {
    const m = await load(
      {
        main: `
          import { Component } from '@angular/core';
          @Component({ selector: 'app', template: '@if (true) { <b>y</b> }' }) export class App {}
        `,
      },
      linker,
    )
    expect(m.App.ɵcmp.template).toBeTypeOf('function')
  })
})

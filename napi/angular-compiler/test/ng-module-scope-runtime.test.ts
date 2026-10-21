/**
 * NgModule scopes in a production build, with the real `@angular/core`.
 *
 * Oxc compiles one file at a time, so a component's directives and pipes are
 * found at runtime by `ɵɵgetComponentDepsFactory`, which reads the scope that
 * `ɵɵsetNgModuleScope` puts on the NgModule. ngtsc guards that call with
 * `ngJitMode` so production builds drop it, and the Vite plugin defines
 * `ngJitMode` as `false` there. If Oxc kept the guard, every component
 * declared in, or importing, an Oxc-compiled NgModule would lose its
 * dependencies in production. These tests run with `ngJitMode = false`.
 */
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { transformAngularFileSync } from '../index.js'

const TMP = join(fileURLToPath(new URL('.', import.meta.url)), '.ng-module-scope-runtime-tmp')
let nextFile = 0
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

/** Compiles `source` and loads it as a module. */
async function load(source: string): Promise<Record<string, any>> {
  const result = transformAngularFileSync(source, '/x/a.ts', {})
  expect(result.errors).toEqual([])
  expect(result.code).not.toContain('ngJitMode')
  const file = join(TMP, `module-${nextFile++}.mjs`)
  writeFileSync(file, result.code)
  return import(/* @vite-ignore */ pathToFileURL(file).href)
}

/** The class names of the directives `cmp`'s template can use. */
function directiveNames(cmp: any): string[] {
  const defs = cmp.ɵcmp.directiveDefs
  return (typeof defs === 'function' ? defs() : (defs ?? [])).map((def: any) => def.type.name)
}

describe('NgModule scope with ngJitMode = false', () => {
  it('gives a declared component the directives of its NgModule', async () => {
    const m = await load(`
      import { Component, Directive, NgModule } from '@angular/core';
      @Directive({ selector: '[d]', standalone: false }) export class D {}
      @Component({ selector: 'c', template: '<i d></i>', standalone: false }) export class C {}
      @NgModule({ declarations: [C, D] }) export class M {}
    `)
    expect(directiveNames(m.C)).toEqual(['C', 'D'])
  })

  it('gives a standalone component the exports of an imported NgModule', async () => {
    const m = await load(`
      import { Component, Directive, NgModule } from '@angular/core';
      @Directive({ selector: '[d]', standalone: false }) export class D {}
      @NgModule({ declarations: [D], exports: [D] }) export class M {}
      @Component({ selector: 's', template: '<i d></i>', imports: [M] }) export class S {}
    `)
    expect(directiveNames(m.S)).toEqual(['S', 'D'])
  })

  it('resolves declarations given as a const array and a spread', async () => {
    const m = await load(`
      import { Component, Directive, NgModule } from '@angular/core';
      @Directive({ selector: '[d]', standalone: false }) export class D {}
      @Component({ selector: 'c', template: '<i d></i>', standalone: false }) export class C {}
      const DIRECTIVES = [D];
      @NgModule({ declarations: [C, ...DIRECTIVES], exports: DIRECTIVES }) export class M {}
      @Component({ selector: 's', template: '<i d></i>', imports: [M] }) export class S {}
    `)
    expect(directiveNames(m.C)).toEqual(['C', 'D'])
    expect(directiveNames(m.S)).toEqual(['S', 'D'])
  })
})

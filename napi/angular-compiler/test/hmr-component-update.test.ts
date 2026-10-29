/**
 * `compileComponentHmrUpdate` compiles an HMR update from the component's
 * source with the options of the full transform, so the template it installs
 * is the template function `transformAngularFile` put in the ɵcmp.
 *
 * Each test compiles a component with the transform, loads it, applies the
 * update module with Angular's own `ɵɵreplaceMetadata`, and compares the
 * result with the definition the transform built.
 */
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import * as core from '@angular/core'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import {
  compileComponentHmrUpdate,
  compileComponentHmrUpdateSync,
  compileForHmrSync,
  transformAngularFileSync,
  type ResolvedResources,
  type TransformOptions,
} from '../index.js'

const TMP = join(fileURLToPath(new URL('.', import.meta.url)), '.hmr-component-update-tmp')
const FILE = '/app/src/x.component.ts'
let nextFile = 0

beforeAll(() => {
  mkdirSync(TMP, { recursive: true })
  // i18n constants call `$localize` when the definition's `consts` are read.
  ;(globalThis as any).$localize ??= (strings: TemplateStringsArray, ...values: unknown[]) =>
    String.raw({ raw: strings }, ...values)
})

afterAll(() => {
  rmSync(TMP, { recursive: true, force: true })
})

/** A component `X` with `metadata` spliced into its decorator. */
function source(metadata: string): string {
  return `import { Component } from '@angular/core';

@Component({
  selector: 'x-cmp',
  ${metadata}
})
export class X {
  name = 'x';
}
`
}

/** Loads an update module the way the browser does: by its default export. */
async function loadUpdateModule(code: string): Promise<(...args: unknown[]) => void> {
  const file = join(TMP, `update-${nextFile++}.mjs`)
  writeFileSync(file, code)
  return (await import(/* @vite-ignore */ pathToFileURL(file).href)).default
}

/** Compiles `code` with the transform and loads the class. */
async function compile(code: string, options: TransformOptions): Promise<any> {
  const result = transformAngularFileSync(code, FILE, options)
  expect(result.errors).toEqual([])
  const file = join(TMP, `component-${nextFile++}.mjs`)
  writeFileSync(file, result.code)
  return (await import(/* @vite-ignore */ pathToFileURL(file).href)).X
}

/**
 * A template function's identity: its source text. A nested function
 * expression carries a trailing `;` that a declaration does not.
 */
function templateSource(fn: unknown): string {
  return String(fn).replace(/};(\s|$)/g, '}$1')
}

function consts(type: any) {
  const def = type.ɵcmp
  return typeof def.consts === 'function' ? def.consts() : def.consts
}

/** `code` without the whitespace the emitter's line wrapping adds. */
function squash(code: string): string {
  return code.replace(/\s+/g, '')
}

/**
 * Compiles `code` with the transform and the HMR update from the same source
 * and options: the update's template function is the one in the transform's
 * output. Then applies the update to a loaded component and checks that the
 * definition's template fields are the transform's.
 */
async function expectUpdateMatchesTransform(code: string, options: TransformOptions = {}) {
  options = { hmr: true, ...options }
  const transformed = transformAngularFileSync(code, FILE, options)
  expect(transformed.errors).toEqual([])
  const result = await compileComponentHmrUpdate(code, FILE, 'X', options)
  expect(result.errors).toEqual([])
  expect(result.hmrModule).not.toBe('')
  expect(squash(transformed.code)).toContain(squash(result.templateJs))

  // Loaded without the HMR initializer, which would fetch `/@ng/component`;
  // `hmr` changes nothing in the ɵcmp.
  const loadOptions = { ...options, hmr: false }
  const fresh = await compile(code, loadOptions)
  const type = await compile(code, loadOptions)
  type.ɵcmp.tView ??= {}
  const applyMetadata = await loadUpdateModule(result.hmrModule)
  ;(core as any).ɵɵreplaceMetadata(type, applyMetadata, [core], [])
  expect(templateSource(type.ɵcmp.template)).toBe(templateSource(result.templateJs))
  expect(type.ɵcmp.decls).toBe(fresh.ɵcmp.decls)
  expect(type.ɵcmp.vars).toBe(fresh.ɵcmp.vars)
  expect(consts(type)).toEqual(consts(fresh))
  expect(type.ɵcmp.styles).toEqual(fresh.ɵcmp.styles)
  expect(type.ɵcmp.tView).toBeNull()
  return result
}

describe('compileComponentHmrUpdate', () => {
  it('keeps the whitespace the decorator preserves', async () => {
    const template = '<div>\n  <span> {{ name }} </span>\n</div>'
    const code = source(`preserveWhitespaces: true, template: ${JSON.stringify(template)},`)
    const result = await expectUpdateMatchesTransform(code)
    expect(result.templateJs).toContain('ɵɵtext(1,"\\n  ")')

    // The template-only compile does not see the decorator.
    const old = compileForHmrSync(template, 'X', FILE, null, { hmr: true })
    expect(templateSource(old.templateJs)).not.toBe(templateSource(result.templateJs))
  })

  it('names i18n messages after the component file', async () => {
    const template = '<p i18n>Hello {{ name }}</p><b i18n-title title="Hi">x</b>'
    const code = source(`template: ${JSON.stringify(template)},`)
    const result = await expectUpdateMatchesTransform(code)
    // Closure message names are derived from the file path. They only show
    // in the Closure branch, which `$localize` skips at runtime.
    const names = (code: string) => code.match(/MSG_\w+/g)?.sort()
    const transformed = transformAngularFileSync(code, FILE, { hmr: true })
    expect(names(result.hmrModule)).toEqual([
      'MSG__APP_SRC_X_COMPONENT_TS_0',
      'MSG__APP_SRC_X_COMPONENT_TS_0',
      'MSG__APP_SRC_X_COMPONENT_TS_1',
      'MSG__APP_SRC_X_COMPONENT_TS_1',
    ])
    expect(names(result.hmrModule)).toEqual(names(transformed.code))

    // The template-only compile has no file to name them after.
    const old = compileForHmrSync(template, 'X', FILE, null, { hmr: true })
    expect(names(old.hmrModule)).not.toEqual(names(result.hmrModule))
  })

  it('locates a template error in the component file, like the transform', async () => {
    // An inline template is part of the source: `file:line:column` in it.
    const code = source('template: `<div>\n  <span>\n</div>`,')
    const transformed = transformAngularFileSync(code, FILE, { hmr: true })
    const result = compileComponentHmrUpdateSync(code, FILE, 'X', { hmr: true })
    expect(result.hmrModule).toBe('')
    const locations = result.errors.map((e) => e.helpMessage)
    expect(locations).toEqual([`${FILE}:7:1`])
    expect(locations).toEqual(transformed.errors.map((e) => e.helpMessage))

    // An external template: `templateUrl:line:column`.
    const external = source(`templateUrl: './x.html',`)
    const resources = {
      templates: { './x.html': '<b>\n</i>' },
      styles: {},
    } as unknown as ResolvedResources
    const update = compileComponentHmrUpdateSync(external, FILE, 'X', { hmr: true }, resources)
    expect(update.errors.map((e) => e.helpMessage)).toEqual(['./x.html:2:1'])
    expect(update.errors.map((e) => e.helpMessage)).toEqual(
      transformAngularFileSync(external, FILE, { hmr: true }, resources).errors.map(
        (e) => e.helpMessage,
      ),
    )

    // The template-only compile has no file to point at.
    const old = compileForHmrSync('<b>\n</i>', 'X', FILE, null, { hmr: true })
    expect(old.errors.map((e) => e.helpMessage)).toEqual(['line 2, column 1 of the template'])
  })

  it('takes the template and styles from the resources, links included', async () => {
    const code = source(
      `templateUrl: './x.html', styleUrl: './x.css', styles: ['b { color: blue }'],`,
    )
    const resources = {
      templates: { './x.html': '<link rel="stylesheet" href="./link.css"><b>{{ name }}</b>' },
      styles: { './x.css': ['b { color: red }'], './link.css': ['b { color: green }'] },
    } as unknown as ResolvedResources
    const fresh = transformAngularFileSync(code, FILE, { hmr: true }, resources)
    expect(fresh.errors).toEqual([])

    const result = compileComponentHmrUpdateSync(code, FILE, 'X', { hmr: true }, resources)
    expect(result.errors).toEqual([])
    expect(result.templateUrl).toBe('./x.html')
    expect(result.styleUrls).toEqual(['./x.css'])
    expect(result.templateStyleUrls).toEqual(['./link.css'])
    expect(result.missingStyleUrls).toEqual([])
    expect(result.styles).toEqual([
      'b[_ngcontent-%COMP%] { color: blue }',
      'b[_ngcontent-%COMP%] { color: red }',
      'b[_ngcontent-%COMP%] { color: green }',
    ])
    for (const style of result.styles!) {
      expect(fresh.code).toContain(JSON.stringify(style))
    }
  })

  it('names the resources it was not given', () => {
    const code = source(`templateUrl: './x.html', styleUrls: ['./x.css'],`)
    const first = compileComponentHmrUpdateSync(code, FILE, 'X', { hmr: true })
    expect(first.hmrModule).toBe('')
    expect(first.errors).toEqual([])
    expect(first.missingTemplateUrl).toBe('./x.html')
    expect(first.missingStyleUrls).toEqual(['./x.css'])

    // A stylesheet that is still missing leaves the live styles alone.
    const resources = {
      templates: { './x.html': '<b>x</b>' },
      styles: {},
    } as unknown as ResolvedResources
    const second = compileComponentHmrUpdateSync(code, FILE, 'X', { hmr: true }, resources)
    expect(second.hmrModule).not.toBe('')
    expect(second.styles ?? null).toBeNull()
    expect(second.hmrModule).not.toContain('styles:')
  })
})

import type { Plugin } from 'vite'
import { describe, expect, it } from 'vitest'

import { angular } from '../vite-plugin/index.js'
import { injectDtsDeclarations } from '../vite-plugin/utils/dts.js'

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-lib-button',
    template: '<button><ng-content></ng-content></button>',
    standalone: true,
  })
  export class LibButtonComponent {}
`

describe('injectDtsDeclarations', () => {
  it('splices members into the matching class and adds the i0 import', () => {
    const source = `export declare class LibButtonComponent {\n}\n`
    const out = injectDtsDeclarations(source, [
      {
        className: 'LibButtonComponent',
        members:
          'static ɵfac: i0.ɵɵFactoryDeclaration<LibButtonComponent, never>;\n' +
          'static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent, "app-lib-button", never, {}, {}, never, ["*"], true, never>;',
      },
    ])

    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<LibButtonComponent, never>;')
    expect(out).toContain(
      'static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent, "app-lib-button", never, {}, {}, never, ["*"], true, never>;',
    )
    // Members land inside the class body, before its closing brace.
    const facIdx = out.indexOf('ɵfac')
    const braceIdx = out.indexOf('class LibButtonComponent')
    const closeIdx = out.lastIndexOf('}')
    expect(braceIdx).toBeLessThan(facIdx)
    expect(facIdx).toBeLessThan(closeIdx)
  })

  it('is idempotent — re-running does not duplicate members', () => {
    const source = `export declare class Foo {\n}\n`
    const decls = [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ]
    const once = injectDtsDeclarations(source, decls)
    const twice = injectDtsDeclarations(once, decls)
    expect(twice).toBe(once)
    expect(once.match(/ɵfac/g)).toHaveLength(1)
  })

  it('reuses an existing i0 import instead of adding a second one', () => {
    const source = 'import * as i0 from "@angular/core";\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out.match(/@angular\/core/g)).toHaveLength(1)
  })

  it('keeps the i0 import after leading triple-slash references', () => {
    const source = '/// <reference types="node" />\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out.indexOf('/// <reference')).toBeLessThan(out.indexOf('import * as i0'))
  })

  it('leaves files without a matching class untouched', () => {
    const source = `export declare class Other {\n}\n`
    const out = injectDtsDeclarations(source, [
      { className: 'Missing', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Missing, never>;' },
    ])
    expect(out).toBe(source)
  })
})

describe('angular() dts plugin (#104)', () => {
  function getPlugins(compilationMode: 'full' | 'partial') {
    const plugins = angular({ compilationMode })
    const transform = plugins.find((p) => p.name === '@oxc-angular/vite')
    const dts = plugins.find((p) => p.name === '@oxc-angular/vite-dts')
    if (!transform || !dts) throw new Error('missing plugins')
    return { transform, dts }
  }

  async function runTransform(transform: Plugin) {
    if (!transform.transform || typeof transform.transform === 'function') {
      throw new Error('expected transform handler')
    }
    await transform.transform.handler.call(
      {
        error(message: string) {
          throw new Error(message)
        },
        warn() {},
      } as any,
      COMPONENT_SOURCE,
      'lib-button.component.ts',
    )
  }

  function makeBundle(dtsSource: string) {
    return {
      'index.d.ts': {
        type: 'asset' as const,
        fileName: 'index.d.ts',
        source: dtsSource,
      },
    }
  }

  async function runGenerateBundle(dts: Plugin, bundle: unknown) {
    const hook = dts.generateBundle
    if (!hook) throw new Error('expected generateBundle')
    const fn = typeof hook === 'function' ? hook : hook.handler
    await fn.call({} as any, {} as any, bundle as any, false)
  }

  it('augments .d.ts assets in partial mode', async () => {
    const { transform, dts } = getPlugins('partial')
    await runTransform(transform)

    const bundle = makeBundle('export declare class LibButtonComponent {\n}\n')
    await runGenerateBundle(dts, bundle)

    const out = bundle['index.d.ts'].source as string
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<LibButtonComponent')
    expect(out).toContain('static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent')
  })

  it('does not touch declarations in full (app) mode', async () => {
    const { transform, dts } = getPlugins('full')
    await runTransform(transform)

    const original = 'export declare class LibButtonComponent {\n}\n'
    const bundle = makeBundle(original)
    await runGenerateBundle(dts, bundle)

    expect(bundle['index.d.ts'].source).toBe(original)
  })
})

/**
 * Tests for SSR + HMR interaction (Issue #109).
 *
 * When using @oxc-angular/vite with Nitro or other SSR frameworks, the server-side
 * bundle must NOT contain HMR initialization code that dynamically imports
 * `@ng/component?c=...` virtual modules, because those are only served via
 * HTTP middleware (not `resolveId`/`load` hooks), causing ERR_LOAD_URL.
 *
 * The fix:
 * 1. The transform hook checks `options.ssr` and disables HMR for SSR transforms.
 * 2. `resolveId`/`load` hooks handle `@ng/component` as a safety net, returning
 *    an empty module so the module runner never crashes.
 */
import { parseSync } from 'vite'
import { describe, it, expect } from 'vitest'

import {
  compileForHmrSync,
  generateHmrModule,
  parseComponentId,
  transformAngularFile,
} from '../index.js'

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-root',
    template: '<h1>Hello World</h1>',
  })
  export class AppComponent {}
`

describe('SSR + HMR (Issue #109)', () => {
  it('should inject HMR code when hmr is enabled (client-side)', async () => {
    const result = await transformAngularFile(COMPONENT_SOURCE, 'app.component.ts', {
      hmr: true,
    })

    expect(result.errors).toHaveLength(0)
    // HMR initializer IIFE should be present
    expect(result.code).toContain('ɵɵreplaceMetadata')
    expect(result.code).toContain('import.meta.hot')
    expect(result.code).toContain('angular:component-update')
  })

  it('should NOT inject HMR code when hmr is disabled (SSR-side)', async () => {
    const result = await transformAngularFile(COMPONENT_SOURCE, 'app.component.ts', {
      hmr: false,
    })

    expect(result.errors).toHaveLength(0)
    // No HMR code should be present
    expect(result.code).not.toContain('ɵɵreplaceMetadata')
    expect(result.code).not.toContain('import.meta.hot')
    expect(result.code).not.toContain('angular:component-update')
    expect(result.code).not.toContain('@ng/component')
    // But the component should still be compiled correctly
    expect(result.code).toContain('ɵɵdefineComponent')
    expect(result.code).toContain('AppComponent')
  })

  it('should produce no templateUpdates when hmr is disabled', async () => {
    const result = await transformAngularFile(COMPONENT_SOURCE, 'app.component.ts', {
      hmr: false,
    })

    expect(result.errors).toHaveLength(0)
    expect(Object.keys(result.templateUpdates).length).toBe(0)
  })
})

describe('Vite plugin SSR behavior (Issue #109)', () => {
  it('angular() plugin should pass ssr flag through to disable HMR', async () => {
    // This test validates the contract: when the Vite plugin receives
    // ssr=true in the transform options, it should set hmr=false
    // in the TransformOptions passed to transformAngularFile.
    //
    // The actual Vite plugin integration is tested via the e2e tests,
    // but this validates the underlying compiler respects hmr=false.
    const clientResult = await transformAngularFile(COMPONENT_SOURCE, 'app.component.ts', {
      hmr: true,
    })
    const ssrResult = await transformAngularFile(COMPONENT_SOURCE, 'app.component.ts', {
      hmr: false,
    })

    // Client should have HMR
    expect(clientResult.code).toContain('ɵɵreplaceMetadata')

    // SSR should NOT have HMR
    expect(ssrResult.code).not.toContain('ɵɵreplaceMetadata')

    // Both should have the component definition
    expect(clientResult.code).toContain('ɵɵdefineComponent')
    expect(ssrResult.code).toContain('ɵɵdefineComponent')
  })
})

describe('Component style minification', () => {
  it('should minify encapsulated HMR styles when enabled', () => {
    const result = compileForHmrSync(
      '<div class="container">Hello</div>',
      'AppComponent',
      'app.component.ts',
      ['.container { color: red; background: transparent; }'],
      { minifyComponentStyles: true },
    )

    expect(result.errors).toHaveLength(0)
    expect(result.hmrModule).toContain('.container[_ngcontent-%COMP%]{color:red;background:0 0}')
  })
})

describe('compileForHmrSync with @ in the file path', () => {
  it.each(['node_modules/@scope/pkg/src/a.ts', 'packages/@a/@b/x.ts'])(
    'names the update function after the class for %s',
    (filePath) => {
      const result = compileForHmrSync('<div></div>', 'Foo', filePath, null, {})

      expect(result.componentId).toBe(`${filePath}@Foo`)
      expect(result.hmrModule).toContain(
        'export default function Foo_UpdateMetadata(Foo, ɵɵnamespaces) {',
      )
      expect(parseSync('hmr.js', result.hmrModule).errors).toEqual([])
    },
  )

  it.each([
    ['node_modules/@scope/pkg/src/a.ts@Foo', 'node_modules/@scope/pkg/src/a.ts', 'Foo'],
    ['packages/@a/@b/x.ts@Foo', 'packages/@a/@b/x.ts', 'Foo'],
    ['Foo', 'Foo', ''],
  ])('parseComponentId splits %s on the last @', (id, filePath, className) => {
    expect(parseComponentId(id)).toEqual({ filePath, className })
  })

  it('generateHmrModule names the update function after the class', () => {
    const hmrModule = generateHmrModule('node_modules/@scope/pkg/src/a.ts@Foo', 'null')

    expect(hmrModule).toContain('export default function Foo_UpdateMetadata(Foo, ɵɵnamespaces) {')
    expect(parseSync('hmr.js', hmrModule).errors).toEqual([])
  })

  it('transformAngularFile emits a parseable HMR initializer with the full id', async () => {
    const filePath = 'node_modules/@scope/pkg/src/app.component.ts'
    const result = await transformAngularFile(COMPONENT_SOURCE, filePath, { hmr: true })

    expect(result.errors).toHaveLength(0)
    expect(result.code).toContain(encodeURIComponent(`${filePath}@AppComponent`))
    expect(parseSync('app.component.js', result.code).errors).toEqual([])
  })
})

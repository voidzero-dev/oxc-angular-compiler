import type { Plugin, PluginOption } from 'vite'
import { describe, expect, it } from 'vitest'

import { angular } from '../vite-plugin/index.js'
import type { PluginOptions } from '../vite-plugin/index.js'

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-root',
    template: '<div>Hello</div>',
  })
  export class AppComponent {}
`

function getAngularPlugin(options: PluginOptions = {}): Plugin {
  const plugin = (angular(options) as PluginOption[]).find(
    (candidate): candidate is Plugin =>
      !!candidate &&
      typeof candidate === 'object' &&
      'name' in candidate &&
      candidate.name === '@oxc-angular/vite',
  )

  if (!plugin) {
    throw new Error('Failed to find @oxc-angular/vite plugin')
  }

  return plugin
}

async function transform(options: PluginOptions = {}): Promise<string> {
  const plugin = getAngularPlugin(options)

  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  const result = await plugin.transform.handler.call(
    {
      error(message: string) {
        throw new Error(message)
      },
      warn() {},
    } as any,
    COMPONENT_SOURCE,
    'app.component.ts',
  )

  if (!result || typeof result !== 'object' || !('code' in result)) {
    throw new Error('Expected transform result with code')
  }

  return result.code as string
}

describe('@oxc-angular/vite class metadata default (ngc parity)', () => {
  it('emits ɵsetClassMetadata by default — matching ngc', async () => {
    const code = await transform()

    expect(code).toContain('setClassMetadata')
  })

  it('wraps ɵsetClassMetadata in the ngDevMode guard so production strips it', async () => {
    const code = await transform()

    // The guarded call looks like:
    //   ((typeof ngDevMode === "undefined") || ngDevMode) && i0.ɵsetClassMetadata(...)
    // Assert ngDevMode appears within ~200 chars *before* setClassMetadata, so we
    // don't false-positive against the unrelated setClassDebugInfo guard.
    expect(code).toMatch(/ngDevMode[\s\S]{0,200}?setClassMetadata/)
  })

  it('omits ɵsetClassMetadata when explicitly disabled', async () => {
    const code = await transform({ emitClassMetadata: false })

    expect(code).not.toContain('setClassMetadata')
  })

  it('emits ɵsetClassMetadata when explicitly enabled', async () => {
    const code = await transform({ emitClassMetadata: true })

    expect(code).toContain('setClassMetadata')
  })
})

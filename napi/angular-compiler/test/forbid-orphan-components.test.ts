import type { Plugin, PluginOption } from 'vite'
import { describe, expect, it } from 'vitest'

import { angular } from '../vite-plugin/index.js'
import type { PluginOptions } from '../vite-plugin/index.js'

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-root',
    template: '<div>Hello</div>',
    standalone: false,
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

describe('@oxc-angular/vite forbidOrphanComponents (ngtsc parity)', () => {
  // ngtsc adds `forbidOrphanRendering: true` to every component's debug info
  // only when `forbidOrphanComponents` is set.
  it('leaves debug info without forbidOrphanRendering by default', async () => {
    const code = await transform()

    expect(code).toContain('setClassDebugInfo')
    expect(code).not.toContain('forbidOrphanRendering')
  })

  it('passes the option to the transform', async () => {
    const code = await transform({ forbidOrphanComponents: true })

    expect(code.replace(/\s+/g, '')).toMatch(
      /ɵsetClassDebugInfo\(AppComponent,\{[^}]*forbidOrphanRendering:true\}\)/,
    )
  })
})

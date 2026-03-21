import type { Plugin } from 'vite'
import { describe, expect, it } from 'vitest'

import { angular } from '../vite-plugin/index.js'

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-root',
    template: '<div class="container">Hello</div>',
    styles: ['.container { color: red; background: transparent; }'],
  })
  export class AppComponent {}
`

function getAngularPlugin() {
  const plugin = angular().find((candidate) => candidate.name === '@oxc-angular/vite')

  if (!plugin) {
    throw new Error('Failed to find @oxc-angular/vite plugin')
  }

  return plugin
}

async function callPluginHook<TArgs extends unknown[], TResult>(
  hook:
    | {
        handler: (...args: TArgs) => TResult
      }
    | ((...args: TArgs) => TResult)
    | undefined,
  ...args: TArgs
): Promise<TResult | undefined> {
  if (!hook) {
    return undefined
  }

  if (typeof hook === 'function') {
    return hook(...args)
  }

  return hook.handler(...args)
}

async function transformWithAutoMinify(
  build: { cssMinify?: boolean | string; minify?: boolean | string },
  resolvedBuild: { cssMinify?: boolean | string; minify?: boolean | string },
): Promise<string> {
  const plugin = getAngularPlugin()

  await callPluginHook(
    plugin.config as Plugin['config'],
    { build } as any,
    { command: 'build', mode: 'production' } as any,
  )
  await callPluginHook(
    plugin.configResolved as Plugin['configResolved'],
    {
      build: resolvedBuild,
      isProduction: true,
    } as any,
  )

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

async function transformWithAutoMinifyFromTransformContext(resolvedBuild: {
  cssMinify?: boolean | string
  minify?: boolean | string
}): Promise<string> {
  const plugin = getAngularPlugin()

  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  const result = await plugin.transform.handler.call(
    {
      environment: {
        config: {
          build: resolvedBuild,
        },
      },
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

async function transformWithAutoMinifyFromOutputOptions(
  outputMinify: boolean | string,
  resolvedBuild: { cssMinify?: boolean | string; minify?: boolean | string },
): Promise<string> {
  const plugin = getAngularPlugin()

  await callPluginHook(
    plugin.outputOptions as Plugin['outputOptions'],
    {
      minify: outputMinify,
    } as any,
  )

  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  const result = await plugin.transform.handler.call(
    {
      environment: {
        config: {
          build: resolvedBuild,
        },
      },
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

describe('@oxc-angular/vite auto component style minification', () => {
  it('should prefer inline build.minify when auto is used', async () => {
    const code = await transformWithAutoMinify({ minify: false }, { cssMinify: true, minify: true })

    expect(code).toContain('.container[_ngcontent-%COMP%] { color: red; background: transparent; }')
  })

  it('should prefer inline build.cssMinify when auto is used', async () => {
    const code = await transformWithAutoMinify(
      { cssMinify: false, minify: true },
      { cssMinify: true, minify: true },
    )

    expect(code).toContain('.container[_ngcontent-%COMP%] { color: red; background: transparent; }')
  })

  it('should fall back to resolved config when inline build minify is not set', async () => {
    const code = await transformWithAutoMinify({}, { cssMinify: true, minify: true })

    expect(code).toContain('.container[_ngcontent-%COMP%]{color:red;background:0 0}')
  })

  it('should fall back to transform context build config when config hooks are skipped', async () => {
    const code = await transformWithAutoMinifyFromTransformContext({
      cssMinify: true,
      minify: true,
    })

    expect(code).toContain('.container[_ngcontent-%COMP%]{color:red;background:0 0}')
  })

  it('should prefer output minify when config hooks are skipped', async () => {
    const code = await transformWithAutoMinifyFromOutputOptions(false, {
      cssMinify: true,
      minify: true,
    })

    expect(code).toContain('.container[_ngcontent-%COMP%] { color: red; background: transparent; }')
  })
})

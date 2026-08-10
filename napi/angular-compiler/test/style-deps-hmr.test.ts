/**
 * Tests HMR for transitive style dependencies.
 *
 * A component styleUrl compiled through a CSS preprocessor can pull in shared
 * files (Sass partials via `@use` / `@import` / `meta.load-css`, Less imports,
 * ...). `preprocessCSS` reports them in `deps`; the plugin must invalidate the
 * compiled style and dispatch component HMR when one of them changes, for every
 * component whose style is built on top of it.
 */
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import type { Plugin, ModuleNode, HmrContext } from 'vite'
import { resolveConfig } from 'vite'
import { afterAll, beforeAll, describe, it, expect, vi } from 'vitest'

import { angular } from '../vite-plugin/index.js'

let tempDir: string
let appDir: string
let sharedScssPath: string
let firstComponentPath: string
let secondComponentPath: string

const componentSource = (selector: string, styleUrl: string) => `
  import { Component } from '@angular/core';

  @Component({
    selector: '${selector}',
    template: '<h1>Hello</h1>',
    styleUrls: ['./${styleUrl}'],
  })
  export class AppComponent {}
`

beforeAll(() => {
  // realpath: Sass canonicalizes loaded URLs (macOS /var -> /private/var),
  // and watcher events use canonical paths too.
  tempDir = realpathSync(mkdtempSync(join(tmpdir(), 'style-deps-hmr-test-')))
  appDir = join(tempDir, 'src', 'app')
  mkdirSync(appDir, { recursive: true })

  sharedScssPath = join(appDir, '_shared.scss')
  firstComponentPath = join(appDir, 'first.component.ts')
  secondComponentPath = join(appDir, 'second.component.ts')

  writeFileSync(sharedScssPath, 'h1 { color: red; }')
  writeFileSync(join(appDir, 'first.component.scss'), "@use './shared';")
  writeFileSync(join(appDir, 'second.component.scss'), "@use './shared';")
  writeFileSync(firstComponentPath, componentSource('app-first', 'first.component.scss'))
  writeFileSync(secondComponentPath, componentSource('app-second', 'second.component.scss'))
})

afterAll(() => {
  rmSync(tempDir, { recursive: true, force: true })
})

function getAngularPlugin() {
  const plugin = angular({ liveReload: true }).find(
    (candidate) => candidate.name === '@oxc-angular/vite',
  )

  if (!plugin) {
    throw new Error('Failed to find @oxc-angular/vite plugin')
  }

  return plugin
}

function createMockServer() {
  const wsMessages: any[] = []

  return {
    watcher: {
      unwatch: vi.fn(),
      on: vi.fn(),
      emit: vi.fn(),
    },
    ws: {
      send(msg: any) {
        wsMessages.push(msg)
      },
      on: vi.fn(),
    },
    moduleGraph: {
      getModuleById: vi.fn(() => null),
      invalidateModule: vi.fn(),
    },
    middlewares: {
      use: vi.fn(),
    },
    config: {
      root: tempDir,
    },
    _wsMessages: wsMessages,
  }
}

function createMockHmrContext(file: string, server: any): HmrContext {
  return {
    file,
    timestamp: Date.now(),
    modules: [{ id: file } as ModuleNode],
    read: async () => '',
    server,
  } as HmrContext
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
  if (!hook) return undefined
  if (typeof hook === 'function') return hook(...args)
  return hook.handler(...args)
}

async function setupPluginWithServer(plugin: Plugin) {
  const mockServer = createMockServer()

  await callPluginHook(
    plugin.config as Plugin['config'],
    {} as any,
    {
      command: 'serve',
      mode: 'development',
    } as any,
  )

  // A real resolved config: preprocessCSS needs one to run Sass and report
  // the partials it loaded in `deps`.
  const resolved = await resolveConfig(
    { configFile: false, root: tempDir, logLevel: 'silent' },
    'serve',
  )
  await callPluginHook(plugin.configResolved as Plugin['configResolved'], resolved as any)

  if (typeof plugin.configureServer === 'function') {
    await (plugin.configureServer as Function)(mockServer)
  }

  ;(mockServer as any).__angularWatchTemplate = () => {}

  return mockServer
}

async function transformComponent(plugin: Plugin, source: string, path: string) {
  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  await plugin.transform.handler.call({ error() {}, warn() {} } as any, source, path)
}

function componentUpdateCount(server: any): number {
  return server._wsMessages.filter((msg: any) => msg?.event === 'angular:component-update').length
}

describe('handleHotUpdate for transitive style dependencies', () => {
  it('dispatches HMR to every component whose style uses a changed Sass partial', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    await transformComponent(
      plugin,
      componentSource('app-first', 'first.component.scss'),
      firstComponentPath,
    )
    await transformComponent(
      plugin,
      componentSource('app-second', 'second.component.scss'),
      secondComponentPath,
    )

    const ctx = createMockHmrContext(sharedScssPath, mockServer)
    const result = await (plugin.handleHotUpdate as Function).call(plugin, ctx)

    // Handled by the plugin: no modules left for Vite's default pipeline.
    expect(result).toEqual([])

    // Both owning components received a component-update event.
    const updates = mockServer._wsMessages.filter(
      (msg) => msg?.event === 'angular:component-update',
    )
    const updatedIds = updates.map((msg) => decodeURIComponent(msg.data.id))
    expect(updatedIds.some((id) => id.startsWith(firstComponentPath))).toBe(true)
    expect(updatedIds.some((id) => id.startsWith(secondComponentPath))).toBe(true)
  })

  it('leaves untracked stylesheets to Vite', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const untracked = join(appDir, 'not-a-dep.scss')
    writeFileSync(untracked, 'h2 { color: blue; }')
    const ctx = createMockHmrContext(untracked, mockServer)
    const result = await (plugin.handleHotUpdate as Function).call(plugin, ctx)

    expect(result).toBe(ctx.modules)
  })

  it('re-registers style deps when a style file switches its imports via HMR', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Dedicated fixtures so this test never interferes with the shared ones.
    const stylePath = join(appDir, 'switch.component.scss')
    const firstDepPath = join(appDir, '_switch-shared.scss')
    const secondDepPath = join(appDir, '_switch-other.scss')
    const componentPath = join(appDir, 'switch.component.ts')

    writeFileSync(firstDepPath, 'h1 { color: red; }')
    writeFileSync(secondDepPath, 'h2 { color: green; }')
    writeFileSync(stylePath, "@use './switch-shared';")
    writeFileSync(componentPath, componentSource('app-switch', 'switch.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-switch', 'switch.component.scss'),
      componentPath,
    )

    // Sanity: the initially registered dep is tracked.
    let result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(firstDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(1)

    // Dev edits the style to import a different partial; HMR rebuilds it.
    writeFileSync(stylePath, "@use './switch-other';")
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(stylePath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(2)

    // The newly imported partial must now be registered as a dep: editing it
    // dispatches a third update (previously it fell through to Vite).
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(secondDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(3)
  })

  it('re-registers nested style deps when a partial switches its own imports via HMR', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // style -> partial -> nested import. Dedicated fixtures so this test never
    // interferes with the shared ones.
    const stylePath = join(appDir, 'nested.component.scss')
    const partialPath = join(appDir, '_nested-a.scss')
    const firstDepPath = join(appDir, '_nested-x.scss')
    const secondDepPath = join(appDir, '_nested-y.scss')
    const componentPath = join(appDir, 'nested.component.ts')

    writeFileSync(firstDepPath, 'h1 { color: red; }')
    writeFileSync(secondDepPath, 'h2 { color: green; }')
    writeFileSync(partialPath, "@use './nested-x';")
    writeFileSync(stylePath, "@use './nested-a';")
    writeFileSync(componentPath, componentSource('app-nested', 'nested.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-nested', 'nested.component.scss'),
      componentPath,
    )

    // Sanity: the initially registered transitive dep is tracked.
    let result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(firstDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(1)

    // Dev edits the partial to import a different nested file; HMR rebuilds.
    writeFileSync(partialPath, "@use './nested-y';")
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(partialPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(2)

    // The newly imported nested file must now be registered as a dep: editing
    // it dispatches a third update (previously it fell through to Vite).
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(secondDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(3)
  })

  it('dispatches HMR for a style that is both a shared dep and a direct styleUrl', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // a.component.scss imports shared.component.scss; component B references
    // shared.component.scss directly as its styleUrl.
    const aStylePath = join(appDir, 'a.component.scss')
    const sharedStylePath = join(appDir, 'shared.component.scss')
    const aComponentPath = join(appDir, 'a.component.ts')
    const bComponentPath = join(appDir, 'b.component.ts')

    writeFileSync(aStylePath, "@use './shared.component';")
    writeFileSync(sharedStylePath, 'h1 { color: red; }')
    writeFileSync(aComponentPath, componentSource('app-a', 'a.component.scss'))
    writeFileSync(bComponentPath, componentSource('app-b', 'shared.component.scss'))

    await transformComponent(plugin, componentSource('app-a', 'a.component.scss'), aComponentPath)
    // Transform B last so resourceToComponent[shared] maps to B (the direct
    // styleUrl owner) rather than A (which only imports it).
    await transformComponent(
      plugin,
      componentSource('app-b', 'shared.component.scss'),
      bComponentPath,
    )

    const result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(sharedStylePath, mockServer),
    )
    expect(result).toEqual([])

    // Both roles updated: A via the shared-dep branch, B via the
    // direct-resource branch (previously the early return skipped B).
    const updates = mockServer._wsMessages.filter(
      (msg) => msg?.event === 'angular:component-update',
    )
    const updatedIds = updates.map((msg) => decodeURIComponent(msg.data.id))
    expect(updatedIds.some((id) => id.startsWith(aComponentPath))).toBe(true)
    expect(updatedIds.some((id) => id.startsWith(bComponentPath))).toBe(true)
  })
})

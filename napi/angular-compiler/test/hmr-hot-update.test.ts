/**
 * Tests for handleHotUpdate behavior (Issue #185).
 *
 * The plugin's handleHotUpdate hook must distinguish between:
 * 1. Component resource files (templates/styles) → handled by custom fs.watch, return []
 * 2. Non-component files (global CSS, etc.) → let Vite handle normally
 *
 * Previously, the plugin returned [] for ALL .css/.html files, which swallowed
 * HMR updates for global stylesheets and prevented PostCSS/Tailwind from
 * processing changes.
 */
import { mkdirSync, mkdtempSync, renameSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import type { Plugin, ModuleNode, HmrContext } from 'vite'
import { normalizePath } from 'vite'
import { afterAll, beforeAll, describe, it, expect, vi } from 'vitest'

import { angular } from '../vite-plugin/index.js'

let tempDir: string
let appDir: string
let templatePath: string
let stylePath: string
let componentPath: string

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-root',
    templateUrl: './app.component.html',
    styleUrls: ['./app.component.css'],
  })
  export class AppComponent {}
`

beforeAll(() => {
  tempDir = mkdtempSync(join(tmpdir(), 'hmr-test-'))
  appDir = join(tempDir, 'src', 'app')
  mkdirSync(appDir, { recursive: true })

  templatePath = join(appDir, 'app.component.html')
  stylePath = join(appDir, 'app.component.css')
  componentPath = join(appDir, 'app.component.ts')

  writeFileSync(templatePath, '<h1>Hello</h1>')
  writeFileSync(stylePath, 'h1 { color: red; }')
  writeFileSync(componentPath, COMPONENT_SOURCE)
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
  const unwatchedFiles = new Set<string>()

  return {
    watcher: {
      unwatch(file: string) {
        unwatchedFiles.add(file)
      },
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
    _unwatchedFiles: unwatchedFiles,
  }
}

function createMockHmrContext(
  file: string,
  modules: Partial<ModuleNode>[] = [],
  server?: any,
): HmrContext {
  return {
    file,
    timestamp: Date.now(),
    modules: modules as ModuleNode[],
    read: async () => '',
    server: server ?? createMockServer(),
  } as HmrContext
}

async function callHandleHotUpdate(
  plugin: Plugin,
  ctx: HmrContext,
): Promise<ModuleNode[] | void | undefined> {
  if (typeof plugin.handleHotUpdate === 'function') {
    return (plugin.handleHotUpdate as Function).call(plugin, ctx)
  }
  return undefined
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

/**
 * Set up a plugin through the full Vite lifecycle so that internal state
 * (watchMode, viteServer, resourceToComponent, componentIds) is populated.
 */
async function setupPluginWithServer(plugin: Plugin) {
  const mockServer = createMockServer()

  // config() sets watchMode = true when command === 'serve'
  await callPluginHook(
    plugin.config as Plugin['config'],
    {} as any,
    {
      command: 'serve',
      mode: 'development',
    } as any,
  )

  // configResolved() stores the resolved config
  await callPluginHook(
    plugin.configResolved as Plugin['configResolved'],
    {
      build: {},
      isProduction: false,
    } as any,
  )

  // configureServer() sets up the custom watcher and stores viteServer
  if (typeof plugin.configureServer === 'function') {
    await (plugin.configureServer as Function)(mockServer)
  }

  // Replace the real fs.watch-based watcher with a no-op to avoid EPERM
  // errors on Windows when temp files are cleaned up. resourceToComponent
  // is populated in transform *before* watchFn is called, so the map is
  // still correctly populated for handleHotUpdate tests.
  ;(mockServer as any).__angularWatchTemplate = () => {}

  return mockServer
}

/**
 * Transform a component that references external template + style files,
 * populating resourceToComponent and componentIds.
 */
async function transformComponent(plugin: Plugin) {
  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  await plugin.transform.handler.call(
    { error() {}, warn() {} } as any,
    COMPONENT_SOURCE,
    componentPath,
  )
}

/**
 * Invoke the Angular component middleware with a synthetic req/res pair and
 * return the response body. Resolves once `res.end()` is called.
 */
async function invokeAngularMiddleware(
  middleware: (...args: any[]) => void,
  componentId: string,
): Promise<string> {
  const encoded = encodeURIComponent(componentId)
  const req = { url: `/@ng/component?c=${encoded}&t=${Date.now()}` }
  let responseBody = ''
  const res = {
    setHeader() {},
    statusCode: 200,
    end(data: string = '') {
      responseBody = data ?? ''
    },
  }
  await new Promise<void>((resolve) => {
    const wrappedRes = {
      ...res,
      end(data: string = '') {
        res.end(data)
        resolve()
      },
    }
    middleware(req, wrappedRes, resolve)
  })
  return responseBody
}

describe('pendingHmrUpdates race condition', () => {
  it('preserves pending entry when template file is transiently empty (truncate-then-write race)', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // Trigger handleHotUpdate for the template → adds componentFile to pendingHmrUpdates
    const componentHtmlFile = normalizePath(templatePath)
    const ctx = createMockHmrContext(componentHtmlFile, [{ id: componentHtmlFile }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    // Extract the encoded component ID that handleHotUpdate broadcast via WS
    const updateMsg = mockServer._wsMessages.find(
      (m: any) => m.event === 'angular:component-update',
    )
    expect(updateMsg, 'expected angular:component-update to be dispatched').toBeDefined()
    const componentId = decodeURIComponent(updateMsg.data.id)

    const middleware = (mockServer.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]
    expect(middleware, 'expected middleware to be registered').toBeDefined()

    // Simulate the truncate phase: file is transiently empty
    writeFileSync(templatePath, '')

    // First HTTP request — file is empty, must return '' but MUST NOT consume
    // the pending entry so the next request can serve real content.
    const firstBody = await invokeAngularMiddleware(middleware, componentId)
    expect(firstBody).toBe('')

    // Restore real content (simulates the second write completing)
    writeFileSync(templatePath, '<h1>Hello</h1>')

    // Second HTTP request — pending entry must still be present → HMR module returned
    const secondBody = await invokeAngularMiddleware(middleware, componentId)
    expect(secondBody, 'expected HMR module to be returned on second request').not.toBe('')

    // Third request — pending entry must have been consumed by the second request
    const thirdBody = await invokeAngularMiddleware(middleware, componentId)
    expect(thirdBody, 'expected pending entry to be consumed after successful HMR').toBe('')
  })

  it('consumes pending entry and dispatches angular:invalidate on compile error', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    const componentHtmlFile = normalizePath(templatePath)
    const ctx = createMockHmrContext(componentHtmlFile, [{ id: componentHtmlFile }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    const updateMsg = mockServer._wsMessages.find(
      (m: any) => m.event === 'angular:component-update',
    )
    expect(updateMsg, 'expected angular:component-update to be dispatched').toBeDefined()
    const componentId = decodeURIComponent(updateMsg.data.id)

    const middleware = (mockServer.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]
    expect(middleware, 'expected middleware to be registered').toBeDefined()

    // Rename the component .ts file so readFile(resolvedId) throws ENOENT,
    // guaranteeing the catch path fires without corrupting templatePath.
    const hiddenPath = componentPath + '.hidden'
    renameSync(componentPath, hiddenPath)
    try {
      const errorBody = await invokeAngularMiddleware(middleware, componentId)
      expect(errorBody).toBe('')

      // angular:invalidate must have been dispatched
      expect(mockServer._wsMessages).toContainEqual(
        expect.objectContaining({ type: 'custom', event: 'angular:invalidate' }),
      )

      // Pending entry must have been consumed — subsequent request returns ''
      const afterErrorBody = await invokeAngularMiddleware(middleware, componentId)
      expect(afterErrorBody, 'expected pending entry to be consumed after error').toBe('')
    } finally {
      renameSync(hiddenPath, componentPath)
    }
  })
})

describe('handleHotUpdate - Issue #185', () => {
  it('should let non-component CSS files pass through to Vite HMR', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    // A global CSS file (not referenced by any component's styleUrls)
    const globalCssFile = normalizePath(join(tempDir, 'src', 'styles.css'))
    const mockModules = [{ id: globalCssFile }]
    const ctx = createMockHmrContext(globalCssFile, mockModules)

    const result = await callHandleHotUpdate(plugin, ctx)

    // Non-component CSS should NOT be swallowed — either undefined (pass through)
    // or the original modules array, but NOT an empty array
    if (result !== undefined) {
      expect(result).toEqual(mockModules)
    }
  })

  it('should dispatch angular:component-update for component CSS files', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // The component's CSS file IS in resourceToComponent
    const componentCssFile = normalizePath(stylePath)
    const mockModules = [{ id: componentCssFile }]
    const ctx = createMockHmrContext(componentCssFile, mockModules, mockServer)

    const result = await callHandleHotUpdate(plugin, ctx)

    // Component resources MUST be swallowed (return []) and dispatch HMR.
    expect(result).toEqual([])
    expect(mockServer._wsMessages).toContainEqual(
      expect.objectContaining({ type: 'custom', event: 'angular:component-update' }),
    )
  })

  it('should dispatch angular:component-update for component template HTML files', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // The component's HTML template IS in resourceToComponent
    const componentHtmlFile = normalizePath(templatePath)
    const ctx = createMockHmrContext(componentHtmlFile, [{ id: componentHtmlFile }], mockServer)

    const result = await callHandleHotUpdate(plugin, ctx)

    // Component templates MUST be swallowed (return []) and dispatch HMR.
    expect(result).toEqual([])
    expect(mockServer._wsMessages).toContainEqual(
      expect.objectContaining({ type: 'custom', event: 'angular:component-update' }),
    )
  })

  it('should not swallow non-resource HTML files', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    // index.html is NOT a component template
    const indexHtml = normalizePath(join(tempDir, 'index.html'))
    const mockModules = [{ id: indexHtml }]
    const ctx = createMockHmrContext(indexHtml, mockModules)

    const result = await callHandleHotUpdate(plugin, ctx)

    // Non-component HTML should pass through, not be swallowed
    if (result !== undefined) {
      expect(result).toEqual(mockModules)
    }
  })

  it('should trigger full-reload for plain (non-component) .ts files', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // src/utils.ts is a plain TS module — Angular's runtime HMR can't
    // refresh captured module bindings, so the only correct fallback is a
    // full reload (matches Angular CLI behavior).
    const utilFile = normalizePath(join(tempDir, 'src', 'utils.ts'))
    const mockModules = [{ id: utilFile }]
    const ctx = createMockHmrContext(utilFile, mockModules, mockServer)

    const result = await callHandleHotUpdate(plugin, ctx)

    expect(result).toEqual([])
    expect(mockServer._wsMessages).toContainEqual(expect.objectContaining({ type: 'full-reload' }))
  })

  it('should ignore .ts files in node_modules', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const depFile = normalizePath(join(tempDir, 'node_modules', 'foo', 'index.ts'))
    const mockModules = [{ id: depFile }]
    const ctx = createMockHmrContext(depFile, mockModules, mockServer)

    const result = await callHandleHotUpdate(plugin, ctx)

    // Should fall through to Vite's default HMR — never trigger a reload
    // for vendor code.
    expect(mockServer._wsMessages).not.toContainEqual(
      expect.objectContaining({ type: 'full-reload' }),
    )
    if (result !== undefined) {
      expect(result).toEqual(mockModules)
    }
  })

  it('should not act when liveReload is disabled', async () => {
    const plugin = angular({ liveReload: false }).find(
      (candidate) => candidate.name === '@oxc-angular/vite',
    )!
    const mockServer = await setupPluginWithServer(plugin)

    const utilFile = normalizePath(join(tempDir, 'src', 'utils.ts'))
    const ctx = createMockHmrContext(utilFile, [{ id: utilFile }], mockServer)

    await callHandleHotUpdate(plugin, ctx)

    // No HMR or full-reload should be sent when liveReload is off.
    expect(mockServer._wsMessages).toHaveLength(0)
  })
})

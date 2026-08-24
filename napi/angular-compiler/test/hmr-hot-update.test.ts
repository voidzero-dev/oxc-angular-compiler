/**
 * Tests for handleHotUpdate behavior (Issue #185).
 *
 * The plugin's handleHotUpdate hook must distinguish between:
 * 1. Component resource files (templates/styles) → dispatch component HMR and
 *    keep Vite's modules flowing (a resource can also be imported by a global
 *    stylesheet, which must still hot-update)
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
import { normalizePath, resolveConfig } from 'vite'
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

function getAngularPlugin(options: Parameters<typeof angular>[0] = { liveReload: true }) {
  const plugin = angular(options).find((candidate) => candidate.name === '@oxc-angular/vite')

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

/**
 * Mock of Vite's mixed module node.
 *
 * `isSelfAccepting` is a prototype getter with no setter on the real node, so
 * the plugin writes the flag to the client-environment node it delegates to.
 * The returned `clientModule` is that node, so a test can assert on it.
 */
function createMockTemplateModule(
  id: string,
  file: string = id,
): {
  module: Partial<ModuleNode>
  clientModule: { isSelfAccepting: boolean }
} {
  const clientModule = { isSelfAccepting: false }
  return {
    module: { id, file, _clientModule: clientModule } as unknown as Partial<ModuleNode>,
    clientModule,
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

  const watched: string[] = []
  await plugin.transform.handler.call(
    {
      error() {},
      warn() {},
      addWatchFile(id: string) {
        watched.push(normalizePath(id))
      },
    } as any,
    COMPONENT_SOURCE,
    componentPath,
  )
  return watched
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

  it('dispatches an HMR event per component when a multi-component .ts file changes (inline template edit)', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Two @Component classes in one file with inline templates.
    const multiComponentPath = join(appDir, 'multi.component.ts')
    const originalSource = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-first', template: '<div>First</div>' })
      export class FirstComponent {}
      @Component({ selector: 'app-second', template: '<div>Second</div>' })
      export class SecondComponent {}
    `
    writeFileSync(multiComponentPath, originalSource)

    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      originalSource,
      multiComponentPath,
    )

    // Edit only FirstComponent's template — stripped form should match the
    // cached stripped form, so the HMR (not full-reload) branch fires.
    const editedSource = originalSource.replace('<div>First</div>', '<div>First Edited</div>')
    writeFileSync(multiComponentPath, editedSource)

    const ctx = createMockHmrContext(multiComponentPath, [{ id: multiComponentPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    // Both components in the file should receive an HMR event (we
    // conservatively dispatch all components when the strip-equality check
    // passes — per-component diffing is a future optimization).
    const updateMsgs = mockServer._wsMessages.filter(
      (m: any) => m.event === 'angular:component-update',
    )
    const componentIds = updateMsgs.map((m: any) => decodeURIComponent(m.data.id))
    expect(componentIds).toContain(`${multiComponentPath}@FirstComponent`)
    expect(componentIds).toContain(`${multiComponentPath}@SecondComponent`)

    // The plugin must NOT have dispatched a full reload.
    const fullReload = mockServer._wsMessages.find((m: any) => m.type === 'full-reload')
    expect(fullReload, 'expected no full-reload for inline template change').toBeUndefined()
  })

  it('serves the correct per-component HMR module via the @ng/component endpoint', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const multiComponentPath = join(appDir, 'multi-endpoint.component.ts')
    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-a', template: '<p>A</p>' })
      export class AComponent {}
      @Component({ selector: 'app-b', template: '<p>B</p>' })
      export class BComponent {}
    `
    writeFileSync(multiComponentPath, source)

    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      multiComponentPath,
    )

    // Trigger a hot update so both components are queued in pendingHmrUpdates.
    writeFileSync(multiComponentPath, source.replace('<p>A</p>', '<p>A!</p>'))
    const ctx = createMockHmrContext(multiComponentPath, [{ id: multiComponentPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    const middleware = (mockServer.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]
    expect(middleware, 'expected middleware to be registered').toBeDefined()

    // Request the HMR module for BOTH components — each should resolve with
    // a non-empty payload that mentions its own className.
    const aBody = await invokeAngularMiddleware(middleware, `${multiComponentPath}@AComponent`)
    const bBody = await invokeAngularMiddleware(middleware, `${multiComponentPath}@BComponent`)

    expect(aBody, 'expected non-empty HMR body for AComponent').not.toBe('')
    expect(bBody, 'expected non-empty HMR body for BComponent').not.toBe('')
    expect(aBody).toContain('AComponent')
    expect(bBody).toContain('BComponent')
  })

  it("dispatches HMR for both components when only one component's inline styles change", async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const multiStylesPath = join(appDir, 'multi-styles.component.ts')
    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-x', template: '<x/>', styles: ['.x { color: red }'] })
      export class XComponent {}
      @Component({ selector: 'app-y', template: '<y/>', styles: ['.y { color: blue }'] })
      export class YComponent {}
    `
    writeFileSync(multiStylesPath, source)

    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      multiStylesPath,
    )

    // Edit only YComponent's styles. Stripping wipes BOTH components' styles
    // (and templates), so old and new stripped forms must still match.
    writeFileSync(multiStylesPath, source.replace('.y { color: blue }', '.y { color: green }'))

    const ctx = createMockHmrContext(multiStylesPath, [{ id: multiStylesPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    const componentIds = mockServer._wsMessages
      .filter((m: any) => m.event === 'angular:component-update')
      .map((m: any) => decodeURIComponent(m.data.id))
    expect(componentIds).toContain(`${multiStylesPath}@XComponent`)
    expect(componentIds).toContain(`${multiStylesPath}@YComponent`)

    const fullReload = mockServer._wsMessages.find((m: any) => m.type === 'full-reload')
    expect(fullReload, 'expected no full-reload for inline styles change').toBeUndefined()
  })

  it('external templateUrl HMR fans out to every component in a multi-component file', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Two components in one file, each with its own templateUrl.
    const externalDir = appDir
    const firstHtmlPath = join(externalDir, 'first.component.html')
    const secondHtmlPath = join(externalDir, 'second.component.html')
    const multiUrlPath = join(externalDir, 'multi-url.component.ts')
    writeFileSync(firstHtmlPath, '<p>First</p>')
    writeFileSync(secondHtmlPath, '<p>Second</p>')

    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-first', templateUrl: './first.component.html' })
      export class FirstComponent {}
      @Component({ selector: 'app-second', templateUrl: './second.component.html' })
      export class SecondComponent {}
    `
    writeFileSync(multiUrlPath, source)

    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      multiUrlPath,
    )

    // Edit just first.component.html. resourceToComponent maps it to
    // multi-url.component.ts; dispatchAllComponentsInFile must fan out to
    // BOTH FirstComponent and SecondComponent (over-dispatch is safe —
    // Angular's runtime no-ops replaceMetadata when nothing changed).
    writeFileSync(firstHtmlPath, '<p>First edited</p>')
    const ctx = createMockHmrContext(
      normalizePath(firstHtmlPath),
      [{ id: normalizePath(firstHtmlPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const componentIds = mockServer._wsMessages
      .filter((m: any) => m.event === 'angular:component-update')
      .map((m: any) => decodeURIComponent(m.data.id))
    expect(componentIds).toContain(`${multiUrlPath}@FirstComponent`)
    expect(componentIds).toContain(`${multiUrlPath}@SecondComponent`)
  })

  it('consumes a stale pending slot when the requested className is no longer in the file', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const stalePath = join(appDir, 'stale.component.ts')
    const originalSource = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-keep', template: '<keep/>' })
      export class KeepComponent {}
      @Component({ selector: 'app-drop', template: '<drop/>' })
      export class DropComponent {}
    `
    writeFileSync(stalePath, originalSource)

    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      originalSource,
      stalePath,
    )

    // Trigger an HMR-eligible edit so a pending entry is queued for both
    // components (including DropComponent).
    writeFileSync(stalePath, originalSource.replace('<keep/>', '<keep-edited/>'))
    const ctx = createMockHmrContext(stalePath, [{ id: stalePath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    // Now re-transform with DropComponent removed. The prune logic must drop
    // the cached pending entry for DropComponent so a later request for it
    // doesn't loop.
    const reducedSource = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-keep', template: '<keep-edited/>' })
      export class KeepComponent {}
    `
    writeFileSync(stalePath, reducedSource)
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      reducedSource,
      stalePath,
    )

    const middleware = (mockServer.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]

    // A request for the now-gone DropComponent must return '' and NOT trigger
    // a phantom HMR module / error / invalidate event.
    const dropBody = await invokeAngularMiddleware(middleware, `${stalePath}@DropComponent`)
    expect(dropBody).toBe('')
    // A second request must also return '' (pending slot consumed first time).
    const dropBody2 = await invokeAngularMiddleware(middleware, `${stalePath}@DropComponent`)
    expect(dropBody2).toBe('')
  })

  it('triggers full reload when a multi-component .ts changes outside template/styles', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const multiReloadPath = join(appDir, 'multi-reload.component.ts')
    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-r1', template: '<r1/>' })
      export class R1Component { value = 1; }
      @Component({ selector: 'app-r2', template: '<r2/>' })
      export class R2Component {}
    `
    writeFileSync(multiReloadPath, source)

    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      multiReloadPath,
    )

    // Change a class member, NOT the template/styles. The stripped form will
    // differ from the cached one → full reload, no HMR.
    writeFileSync(multiReloadPath, source.replace('value = 1', 'value = 2'))

    const ctx = createMockHmrContext(multiReloadPath, [{ id: multiReloadPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    const componentUpdates = mockServer._wsMessages.filter(
      (m: any) => m.event === 'angular:component-update',
    )
    expect(
      componentUpdates,
      'expected no component-update events for non-template change',
    ).toHaveLength(0)

    const fullReload = mockServer._wsMessages.find((m: any) => m.type === 'full-reload')
    expect(fullReload, 'expected a full-reload event').toBeDefined()
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

    // Component HMR is dispatched, and Vite's modules are preserved for the
    // default pipeline (e.g. a global stylesheet importing the same file).
    expect(result).toEqual(mockModules)
    expect(mockServer._wsMessages).toContainEqual(
      expect.objectContaining({ type: 'custom', event: 'angular:component-update' }),
    )
  })

  it('should dispatch angular:component-update for component template HTML files', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // The component's HTML template IS in resourceToComponent.
    // `_clientModule` mirrors Vite's mixed module node: `isSelfAccepting` is
    // a prototype getter there, so the flag lands on the client node.
    const componentHtmlFile = normalizePath(templatePath)
    const { module: templateModule, clientModule } = createMockTemplateModule(componentHtmlFile)
    const ctx = createMockHmrContext(componentHtmlFile, [templateModule], mockServer)

    const result = await callHandleHotUpdate(plugin, ctx)

    // Component HMR is dispatched, and the template module becomes its own
    // HMR boundary. Without that, Vite sends a `full-reload` for the changed
    // `.html` on top of the update we just dispatched — and that reload is
    // unconditional in `middlewareMode`, where its path is `*`.
    // See https://github.com/voidzero-dev/oxc-angular-compiler/issues/443.
    expect(result).toEqual([templateModule])
    expect(clientModule.isSelfAccepting).toBe(true)
    expect(mockServer._wsMessages).toContainEqual(
      expect.objectContaining({ type: 'custom', event: 'angular:component-update' }),
    )
  })

  it('should not mark a postfixed variant of the template as self-accepting', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // Vite sets `file` to `cleanUrl(id)`, so a browser-imported
    // `./app.component.html?raw` is filed under the template's path and shows
    // up in `ctx.modules` alongside the node `addWatchFile` created.
    const componentHtmlFile = normalizePath(templatePath)
    const bare = createMockTemplateModule(componentHtmlFile)
    const raw = createMockTemplateModule(`${componentHtmlFile}?raw`, componentHtmlFile)
    const ctx = createMockHmrContext(componentHtmlFile, [bare.module, raw.module], mockServer)

    await callHandleHotUpdate(plugin, ctx)

    // Only the template itself becomes the boundary.
    expect(bare.clientModule.isSelfAccepting).toBe(true)
    // The `?raw` variant has real importers holding its value. Stopping
    // propagation there would leave them with a stale string, because the
    // browser has no `import.meta.hot.accept` handler for it.
    expect(raw.clientModule.isSelfAccepting).toBe(false)
  })

  it('should not mark non-component HTML as self-accepting', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    // index.html must keep reloading the page — it is not hot-swappable.
    const indexHtml = normalizePath(join(tempDir, 'index.html'))
    const { module: indexModule, clientModule } = createMockTemplateModule(indexHtml)
    const ctx = createMockHmrContext(indexHtml, [indexModule])

    const result = await callHandleHotUpdate(plugin, ctx)

    expect(result).toEqual([indexModule])
    expect(clientModule.isSelfAccepting).toBe(false)
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
    const plugin = getAngularPlugin({ liveReload: false })
    const mockServer = await setupPluginWithServer(plugin)

    const utilFile = normalizePath(join(tempDir, 'src', 'utils.ts'))
    const ctx = createMockHmrContext(utilFile, [{ id: utilFile }], mockServer)

    await callHandleHotUpdate(plugin, ctx)

    // No HMR or full-reload should be sent when liveReload is off.
    expect(mockServer._wsMessages).toHaveLength(0)
  })

  it('should not add template graph edges when liveReload is disabled', async () => {
    const plugin = getAngularPlugin({ liveReload: false })
    await setupPluginWithServer(plugin)

    const watched = await transformComponent(plugin)

    // The graph edge exists only to keep Vite off its `.html` reload branch
    // during HMR. With HMR off, `handleHotUpdate` returns before it can use
    // the edge, and the edge alone turns Vite's dropped `.html` payload into
    // an unconditional `full-reload` with path `*`.
    expect(watched).toEqual([])
  })

  it('should add a template graph edge when liveReload is enabled', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    const watched = await transformComponent(plugin)

    expect(watched).toContain(normalizePath(templatePath))
  })
})

describe('handleHotUpdate for a templateUrl shared across component files - Issue #445', () => {
  const componentSource = (selector: string, className: string, templateUrl: string) => `
    import { Component } from '@angular/core';
    @Component({ selector: '${selector}', templateUrl: './${templateUrl}' })
    export class ${className} {}
  `

  async function transformSharedComponent(plugin: Plugin, source: string, path: string) {
    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      path,
    )
  }

  function componentUpdateIds(server: any): string[] {
    return server._wsMessages
      .filter((m: any) => m.event === 'angular:component-update')
      .map((m: any) => decodeURIComponent(m.data.id))
  }

  it('dispatches HMR to every component file sharing the template', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const sharedHtmlPath = join(appDir, 'shared-owners.component.html')
    const alphaPath = join(appDir, 'shared-owner-alpha.component.ts')
    const betaPath = join(appDir, 'shared-owner-beta.component.ts')
    writeFileSync(sharedHtmlPath, '<p>Shared</p>')

    const alphaSource = componentSource(
      'app-shared-a',
      'AlphaComponent',
      'shared-owners.component.html',
    )
    const betaSource = componentSource(
      'app-shared-b',
      'BetaComponent',
      'shared-owners.component.html',
    )
    writeFileSync(alphaPath, alphaSource)
    writeFileSync(betaPath, betaSource)

    await transformSharedComponent(plugin, alphaSource, alphaPath)
    await transformSharedComponent(plugin, betaSource, betaPath)

    // Editing the shared template must update BOTH owner files, not just the
    // last-transformed one that resourceToComponent happens to point at.
    writeFileSync(sharedHtmlPath, '<p>Shared edited</p>')
    const ctx = createMockHmrContext(
      normalizePath(sharedHtmlPath),
      [{ id: normalizePath(sharedHtmlPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const componentIds = componentUpdateIds(mockServer)
    expect(componentIds).toContain(`${alphaPath}@AlphaComponent`)
    expect(componentIds).toContain(`${betaPath}@BetaComponent`)
  })

  it('dispatches HMR to remaining owners when one owner switches templates', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const sharedHtmlPath = join(appDir, 'shared-prune.component.html')
    const betaOwnHtmlPath = join(appDir, 'shared-prune-own.component.html')
    const alphaPath = join(appDir, 'prune-owner-alpha.component.ts')
    const betaPath = join(appDir, 'prune-owner-beta.component.ts')
    writeFileSync(sharedHtmlPath, '<p>Shared</p>')
    writeFileSync(betaOwnHtmlPath, '<p>Own</p>')

    const alphaSource = componentSource(
      'app-prune-a',
      'AlphaComponent',
      'shared-prune.component.html',
    )
    const betaSource = componentSource(
      'app-prune-b',
      'BetaComponent',
      'shared-prune.component.html',
    )
    writeFileSync(alphaPath, alphaSource)
    writeFileSync(betaPath, betaSource)

    await transformSharedComponent(plugin, alphaSource, alphaPath)
    await transformSharedComponent(plugin, betaSource, betaPath)

    // Beta switches to its own template; its prune removes the single-valued
    // resourceToComponent entry for the shared template.
    const betaSwitchedSource = componentSource(
      'app-prune-b',
      'BetaComponent',
      'shared-prune-own.component.html',
    )
    writeFileSync(betaPath, betaSwitchedSource)
    await transformSharedComponent(plugin, betaSwitchedSource, betaPath)

    // Editing the shared template must still update Alpha (reachable via
    // templateComponentOwners even though resourceToComponent no longer has it).
    writeFileSync(sharedHtmlPath, '<p>Shared edited</p>')
    const ctx = createMockHmrContext(
      normalizePath(sharedHtmlPath),
      [{ id: normalizePath(sharedHtmlPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const componentIds = componentUpdateIds(mockServer)
    expect(componentIds).toContain(`${alphaPath}@AlphaComponent`)
    expect(componentIds).not.toContain(`${betaPath}@BetaComponent`)
  })

  it('marks the shared template module self-accepting when dispatching to every owner', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const sharedHtmlPath = join(appDir, 'shared-accept.component.html')
    const alphaPath = join(appDir, 'accept-owner-alpha.component.ts')
    const betaPath = join(appDir, 'accept-owner-beta.component.ts')
    writeFileSync(sharedHtmlPath, '<p>Shared</p>')

    const alphaSource = componentSource(
      'app-accept-a',
      'AlphaComponent',
      'shared-accept.component.html',
    )
    const betaSource = componentSource(
      'app-accept-b',
      'BetaComponent',
      'shared-accept.component.html',
    )
    writeFileSync(alphaPath, alphaSource)
    writeFileSync(betaPath, betaSource)

    await transformSharedComponent(plugin, alphaSource, alphaPath)
    await transformSharedComponent(plugin, betaSource, betaPath)

    const normalizedShared = normalizePath(sharedHtmlPath)
    const { module: templateModule, clientModule } = createMockTemplateModule(normalizedShared)
    const ctx = createMockHmrContext(normalizedShared, [templateModule], mockServer)

    const result = await callHandleHotUpdate(plugin, ctx)

    // The multi-owner dispatch must keep the template as its own HMR boundary
    // (issue #443) while updating both owners.
    expect(result).toEqual([templateModule])
    expect(clientModule.isSelfAccepting).toBe(true)
    const componentIds = componentUpdateIds(mockServer)
    expect(componentIds).toContain(`${alphaPath}@AlphaComponent`)
    expect(componentIds).toContain(`${betaPath}@BetaComponent`)
  })

  it('dispatches HMR for a templateUrl that does not end in .html', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // A templateUrl may point at ANY file — resolveResources reads it with no
    // extension check. Ownership must be tracked by ROLE (declared as a
    // templateUrl), not by file extension. This .css file is nobody's styleUrl.
    const oddTemplatePath = join(appDir, 'odd-tpl.view.css')
    const htmlTemplatePath = join(appDir, 'odd-guard.component.html')
    const oddOwnerPath = join(appDir, 'odd-owner.component.ts')
    const htmlOwnerPath = join(appDir, 'odd-guard.component.ts')
    writeFileSync(oddTemplatePath, '<p>ODD_TPL_MARKER</p>')
    writeFileSync(htmlTemplatePath, '<p>ODD_GUARD_MARKER</p>')

    const oddSource = componentSource('app-odd', 'OddComponent', 'odd-tpl.view.css')
    const htmlSource = componentSource(
      'app-odd-guard',
      'HtmlGuardComponent',
      'odd-guard.component.html',
    )
    writeFileSync(oddOwnerPath, oddSource)
    writeFileSync(htmlOwnerPath, htmlSource)

    await transformSharedComponent(plugin, oddSource, oddOwnerPath)
    await transformSharedComponent(plugin, htmlSource, htmlOwnerPath)

    // Editing the .css-named template must dispatch to its owner.
    writeFileSync(oddTemplatePath, '<p>ODD_TPL_MARKER edited</p>')
    await callHandleHotUpdate(
      plugin,
      createMockHmrContext(
        normalizePath(oddTemplatePath),
        [{ id: normalizePath(oddTemplatePath) }],
        mockServer,
      ),
    )
    expect(componentUpdateIds(mockServer)).toContain(`${oddOwnerPath}@OddComponent`)

    // Guard: a plain .html templateUrl still dispatches through the same path.
    writeFileSync(htmlTemplatePath, '<p>ODD_GUARD_MARKER edited</p>')
    await callHandleHotUpdate(
      plugin,
      createMockHmrContext(
        normalizePath(htmlTemplatePath),
        [{ id: normalizePath(htmlTemplatePath) }],
        mockServer,
      ),
    )
    expect(componentUpdateIds(mockServer)).toContain(`${htmlOwnerPath}@HtmlGuardComponent`)
  })
})

describe('handleHotUpdate for a resource used in two decorator roles - Issue #450', () => {
  async function transformDualComponent(plugin: Plugin, source: string, path: string) {
    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      path,
    )
  }

  function componentUpdateIds(server: any): string[] {
    return server._wsMessages
      .filter((m: any) => m.event === 'angular:component-update')
      .map((m: any) => decodeURIComponent(m.data.id))
  }

  const styleOwnerSource = (selector: string, className: string, styleUrl: string) => `
    import { Component } from '@angular/core';
    @Component({ selector: '${selector}', template: '<p>inline</p>', styleUrls: ['./${styleUrl}'] })
    export class ${className} {}
  `

  const templateOwnerSource = (selector: string, className: string, templateUrl: string) => `
    import { Component } from '@angular/core';
    @Component({ selector: '${selector}', templateUrl: './${templateUrl}' })
    export class ${className} {}
  `

  /**
   * One file, two decorator roles: component A's styleUrl and component B's
   * templateUrl. Both owners must receive an update when it changes.
   *
   * The file holds template markup, not CSS: the template owner must compile,
   * and a styleUrl is registered as a direct style regardless of whether
   * preprocessing succeeds.
   */
  async function runDualRoleCase(
    plugin: Plugin,
    mockServer: any,
    prefix: string,
    transformTemplateOwnerFirst: boolean,
  ) {
    const dualPath = join(appDir, `${prefix}-dual.css`)
    const stylePath = join(appDir, `${prefix}-style-owner.component.ts`)
    const templatePath = join(appDir, `${prefix}-template-owner.component.ts`)
    writeFileSync(dualPath, '<p>DUAL_V1</p>')

    const styleSource = styleOwnerSource(
      `app-${prefix}-style`,
      'StyleOwnerComponent',
      `${prefix}-dual.css`,
    )
    const templateSource = templateOwnerSource(
      `app-${prefix}-template`,
      'TemplateOwnerComponent',
      `${prefix}-dual.css`,
    )
    writeFileSync(stylePath, styleSource)
    writeFileSync(templatePath, templateSource)

    if (transformTemplateOwnerFirst) {
      await transformDualComponent(plugin, templateSource, templatePath)
      await transformDualComponent(plugin, styleSource, stylePath)
    } else {
      await transformDualComponent(plugin, styleSource, stylePath)
      await transformDualComponent(plugin, templateSource, templatePath)
    }

    writeFileSync(dualPath, '<p>DUAL_V2</p>')
    await callHandleHotUpdate(
      plugin,
      createMockHmrContext(normalizePath(dualPath), [{ id: normalizePath(dualPath) }], mockServer),
    )

    return { stylePath, templatePath }
  }

  it('dispatches to the style owner and the template owner of the same file', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Template owner first: this order leaves the style owner as the only
    // member of styleComponentOwners, so forking on `isDirectStyle` skipped
    // the template owner entirely and it kept rendering the old template.
    const { stylePath, templatePath } = await runDualRoleCase(plugin, mockServer, 'dr', true)

    const ids = componentUpdateIds(mockServer)
    expect(ids).toContain(`${stylePath}@StyleOwnerComponent`)
    expect(ids).toContain(`${templatePath}@TemplateOwnerComponent`)
  })

  it('dispatches to both owners regardless of transform order', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Style owner first. The role sets are global and monotonic, so the
    // second-transformed file registers in BOTH owner maps and the style loop
    // alone happened to cover both. Pinned so the outcome stays order-independent.
    const { stylePath, templatePath } = await runDualRoleCase(plugin, mockServer, 'dro', false)

    const ids = componentUpdateIds(mockServer)
    expect(ids).toContain(`${stylePath}@StyleOwnerComponent`)
    expect(ids).toContain(`${templatePath}@TemplateOwnerComponent`)
  })

  it('dispatches exactly once per class when one file owns the resource in both roles', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Both classes live in ONE .ts file, so that file is registered as both a
    // style owner and a template owner. Dispatching per role would send two
    // events per class.
    const dualPath = join(appDir, 'same-file-dual.css')
    const ownerPath = join(appDir, 'same-file-owner.component.ts')
    writeFileSync(dualPath, '<p>SAME_FILE_V1</p>')

    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-sf-style', template: '<p>inline</p>', styleUrls: ['./same-file-dual.css'] })
      export class SameFileStyleComponent {}
      @Component({ selector: 'app-sf-template', templateUrl: './same-file-dual.css' })
      export class SameFileTemplateComponent {}
    `
    writeFileSync(ownerPath, source)
    await transformDualComponent(plugin, source, ownerPath)

    writeFileSync(dualPath, '<p>SAME_FILE_V2</p>')
    await callHandleHotUpdate(
      plugin,
      createMockHmrContext(normalizePath(dualPath), [{ id: normalizePath(dualPath) }], mockServer),
    )

    const ids = componentUpdateIds(mockServer)
    expect(ids).toContain(`${ownerPath}@SameFileStyleComponent`)
    expect(ids).toContain(`${ownerPath}@SameFileTemplateComponent`)
    expect(ids.filter((id) => id === `${ownerPath}@SameFileStyleComponent`)).toHaveLength(1)
    expect(ids.filter((id) => id === `${ownerPath}@SameFileTemplateComponent`)).toHaveLength(1)
  })

  it('dispatches exactly once per component for a plain shared styleUrl', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Guard: a style with no template role keeps its existing behavior — one
    // event per owning component, no duplicates.
    const sharedCssPath = join(appDir, 'plain-shared.component.css')
    const firstPath = join(appDir, 'plain-first.component.ts')
    const secondPath = join(appDir, 'plain-second.component.ts')
    writeFileSync(sharedCssPath, 'h1 { color: red; }')

    const firstSource = styleOwnerSource(
      'app-plain-first',
      'PlainFirstComponent',
      'plain-shared.component.css',
    )
    const secondSource = styleOwnerSource(
      'app-plain-second',
      'PlainSecondComponent',
      'plain-shared.component.css',
    )
    writeFileSync(firstPath, firstSource)
    writeFileSync(secondPath, secondSource)
    await transformDualComponent(plugin, firstSource, firstPath)
    await transformDualComponent(plugin, secondSource, secondPath)

    writeFileSync(sharedCssPath, 'h1 { color: blue; }')
    await callHandleHotUpdate(
      plugin,
      createMockHmrContext(
        normalizePath(sharedCssPath),
        [{ id: normalizePath(sharedCssPath) }],
        mockServer,
      ),
    )

    const ids = componentUpdateIds(mockServer)
    expect(ids).toEqual([`${firstPath}@PlainFirstComponent`, `${secondPath}@PlainSecondComponent`])
  })
})

describe('@ng/component endpoint resolves the template per class', () => {
  async function transformSource(plugin: Plugin, source: string, path: string) {
    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      path,
    )
  }

  // The component-file branch queues `pendingHmrUpdates` under `ctx.file`
  // verbatim, and the endpoint looks the id up verbatim. Asserting the queued
  // id here turns a spelling mismatch into a named failure instead of an
  // unexplained empty body — the difference only shows up on Windows, where
  // a normalized path and a `join()` path are different strings.
  function expectDispatched(mockServer: any, componentId: string) {
    const ids = mockServer._wsMessages
      .filter((m: any) => m?.event === 'angular:component-update')
      .map((m: any) => decodeURIComponent(m.data.id))
    expect(ids, 'expected the HMR dispatch to use the requested path spelling').toContain(
      componentId,
    )
  }

  function getMiddleware(mockServer: any) {
    const middleware = (mockServer.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]
    expect(middleware, 'expected middleware to be registered').toBeDefined()
    return middleware
  }

  it('serves each templateUrl component its own template in a multi-templateUrl file', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const firstHtmlPath = join(appDir, 'pc-first.component.html')
    const secondHtmlPath = join(appDir, 'pc-second.component.html')
    const multiUrlPath = join(appDir, 'pc-multi-url.component.ts')
    writeFileSync(firstHtmlPath, '<p>PC_FIRST_MARKER</p>')
    writeFileSync(secondHtmlPath, '<p>PC_SECOND_MARKER</p>')

    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-pc-first', templateUrl: './pc-first.component.html' })
      export class FirstComponent {}
      @Component({ selector: 'app-pc-second', templateUrl: './pc-second.component.html' })
      export class SecondComponent {}
    `
    writeFileSync(multiUrlPath, source)
    await transformSource(plugin, source, multiUrlPath)

    // Edit the SECOND component's template; the fan-out queues both classes.
    writeFileSync(secondHtmlPath, '<p>PC_SECOND_MARKER edited</p>')
    const ctx = createMockHmrContext(
      normalizePath(secondHtmlPath),
      [{ id: normalizePath(secondHtmlPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    // SecondComponent's update module must be compiled from second.html, not
    // from the file's first templateUrl.
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${multiUrlPath}@SecondComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PC_SECOND_MARKER')
    expect(body).not.toContain('PC_FIRST_MARKER')
  })

  it('does not serve the shared template to a sibling with its own template', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const sharedHtmlPath = join(appDir, 'pc-shared.component.html')
    const ownHtmlPath = join(appDir, 'pc-own.component.html')
    const fileAPath = join(appDir, 'pc-owner-a.component.ts')
    const fileBPath = join(appDir, 'pc-owner-b.component.ts')
    writeFileSync(sharedHtmlPath, '<p>PC_SHARED_MARKER</p>')
    writeFileSync(ownHtmlPath, '<p>PC_OWN_MARKER</p>')

    // In fileA the shared template is declared FIRST, so templateUrls[0]
    // points at it for every class in the file.
    const fileASource = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-pc-shared-a', templateUrl: './pc-shared.component.html' })
      export class SharedAComponent {}
      @Component({ selector: 'app-pc-own', templateUrl: './pc-own.component.html' })
      export class OwnComponent {}
    `
    const fileBSource = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-pc-shared-b', templateUrl: './pc-shared.component.html' })
      export class SharedBComponent {}
    `
    writeFileSync(fileAPath, fileASource)
    writeFileSync(fileBPath, fileBSource)
    await transformSource(plugin, fileASource, fileAPath)
    await transformSource(plugin, fileBSource, fileBPath)

    writeFileSync(sharedHtmlPath, '<p>PC_SHARED_MARKER edited</p>')
    const ctx = createMockHmrContext(
      normalizePath(sharedHtmlPath),
      [{ id: normalizePath(sharedHtmlPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    // OwnComponent was queued by the intra-file fan-out; its served module
    // must still be compiled from its OWN template, not the shared one.
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${fileAPath}@OwnComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PC_OWN_MARKER')
    expect(body).not.toContain('PC_SHARED_MARKER')
  })

  it('serves the inline template of a class whose sibling uses a templateUrl', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const extHtmlPath = join(appDir, 'pc-ext.component.html')
    const mixedPath = join(appDir, 'pc-mixed.component.ts')
    writeFileSync(extHtmlPath, '<p>PC_EXT_MARKER</p>')

    const source = `
      import { Component } from '@angular/core';
      @Component({ selector: 'app-pc-ext', templateUrl: './pc-ext.component.html' })
      export class ExtComponent {}
      @Component({ selector: 'app-pc-inline', template: '<p>PC_INLINE_MARKER</p>' })
      export class InlineComponent {}
    `
    writeFileSync(mixedPath, source)
    await transformSource(plugin, source, mixedPath)

    // Edit only the inline template; the strip-equality branch queues both
    // classes in the file.
    const edited = source.replace('PC_INLINE_MARKER', 'PC_INLINE_MARKER edited')
    writeFileSync(mixedPath, edited)
    const ctx = createMockHmrContext(mixedPath, [{ id: mixedPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    expectDispatched(mockServer, `${mixedPath}@InlineComponent`)

    // InlineComponent must get its inline template — templateUrls.length > 0
    // for the FILE must not shadow the per-class inline branch.
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${mixedPath}@InlineComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PC_INLINE_MARKER')
    expect(body).not.toContain('PC_EXT_MARKER')
  })
})

describe('@ng/component endpoint resolves the styles per class', () => {
  // preprocessCSS needs a REAL resolved config to run; the shared mock config
  // makes every external stylesheet fail to preprocess, which would hide what
  // these tests are about. Other tests in this file keep the mock.
  async function setupPluginWithRealConfig(plugin: Plugin) {
    const mockServer = createMockServer()

    await callPluginHook(
      plugin.config as Plugin['config'],
      {} as any,
      { command: 'serve', mode: 'development' } as any,
    )
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

  async function transformSource(plugin: Plugin, source: string, path: string) {
    if (!plugin.transform || typeof plugin.transform === 'function') {
      throw new Error('Expected plugin transform handler')
    }
    await plugin.transform.handler.call(
      { error() {}, warn() {}, addWatchFile() {} } as any,
      source,
      path,
    )
  }

  // The component-file branch queues `pendingHmrUpdates` under `ctx.file`
  // verbatim, and the endpoint looks the id up verbatim. Asserting the queued
  // id here turns a spelling mismatch into a named failure instead of an
  // unexplained empty body — the difference only shows up on Windows, where
  // a normalized path and a `join()` path are different strings.
  function expectDispatched(mockServer: any, componentId: string) {
    const ids = mockServer._wsMessages
      .filter((m: any) => m?.event === 'angular:component-update')
      .map((m: any) => decodeURIComponent(m.data.id))
    expect(ids, 'expected the HMR dispatch to use the requested path spelling').toContain(
      componentId,
    )
  }

  function getMiddleware(mockServer: any) {
    const middleware = (mockServer.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]
    expect(middleware, 'expected middleware to be registered').toBeDefined()
    return middleware
  }

  it('serves each component its own styleUrls in a multi-component file', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const firstCssPath = join(appDir, 'ps-first.component.css')
    const secondCssPath = join(appDir, 'ps-second.component.css')
    const multiPath = join(appDir, 'ps-multi.component.ts')
    writeFileSync(firstCssPath, '.PS_FIRST_MARKER { color: red; }')
    writeFileSync(secondCssPath, '.PS_SECOND_MARKER { color: blue; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-first',
        template: '<p>first</p>',
        styleUrls: ['./ps-first.component.css'],
      })
      export class FirstComponent {}
      @Component({
        selector: 'app-ps-second',
        template: '<p>second</p>',
        styleUrls: ['./ps-second.component.css'],
      })
      export class SecondComponent {}
    `
    writeFileSync(multiPath, source)
    await transformSource(plugin, source, multiPath)

    // Edit the SECOND component's stylesheet; the fan-out queues both classes.
    writeFileSync(secondCssPath, '.PS_SECOND_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(secondCssPath),
      [{ id: normalizePath(secondCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    // SecondComponent's update module must carry only its OWN stylesheet, not
    // the union of every styleUrl declared in the file.
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${multiPath}@SecondComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_SECOND_MARKER')
    expect(body).not.toContain('PS_FIRST_MARKER')
  })

  it('serves the singular `styleUrl` of the requested class', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const soloCssPath = join(appDir, 'ps-solo.component.css')
    const otherCssPath = join(appDir, 'ps-other.component.css')
    const singularPath = join(appDir, 'ps-singular.component.ts')
    writeFileSync(soloCssPath, '.PS_SOLO_MARKER { color: red; }')
    writeFileSync(otherCssPath, '.PS_OTHER_MARKER { color: blue; }')

    // `styleUrl` (singular, Angular 17+) is a bare string, not an array.
    // It is declared SECOND here so the file-level list does not happen to
    // start with it.
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-other',
        template: '<p>other</p>',
        styleUrl: './ps-other.component.css',
      })
      export class OtherComponent {}
      @Component({
        selector: 'app-ps-solo',
        template: '<p>solo</p>',
        styleUrl: './ps-solo.component.css',
      })
      export class SoloComponent {}
    `
    writeFileSync(singularPath, source)
    await transformSource(plugin, source, singularPath)

    writeFileSync(soloCssPath, '.PS_SOLO_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(soloCssPath),
      [{ id: normalizePath(soloCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${singularPath}@SoloComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_SOLO_MARKER')
    expect(body).not.toContain('PS_OTHER_MARKER')
  })

  it('serves the inline styles of a class whose sibling uses a styleUrl', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const extCssPath = join(appDir, 'ps-ext.component.css')
    const mixedPath = join(appDir, 'ps-mixed.component.ts')
    writeFileSync(extCssPath, '.PS_EXT_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-ext',
        template: '<p>ext</p>',
        styleUrls: ['./ps-ext.component.css'],
      })
      export class ExtComponent {}
      @Component({
        selector: 'app-ps-inline',
        template: '<p>inline</p>',
        styles: ['.PS_INLINE_MARKER { color: blue; }'],
      })
      export class InlineComponent {}
    `
    writeFileSync(mixedPath, source)
    await transformSource(plugin, source, mixedPath)

    // Edit only the inline styles; the strip-equality branch queues both
    // classes in the file.
    const edited = source.replace('color: blue', 'color: green')
    writeFileSync(mixedPath, edited)
    const ctx = createMockHmrContext(mixedPath, [{ id: mixedPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    expectDispatched(mockServer, `${mixedPath}@InlineComponent`)

    // InlineComponent must get its OWN inline styles — a sibling's styleUrl
    // making the FILE-level list non-empty must not shadow the inline branch.
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${mixedPath}@InlineComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_INLINE_MARKER')
    expect(body).not.toContain('PS_EXT_MARKER')
  })
  it('serves the styleUrls of a class whose array carries a comment', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const realCssPath = join(appDir, 'ps-commented.component.css')
    const commentedPath = join(appDir, 'ps-commented.component.ts')
    writeFileSync(realCssPath, '.PS_COMMENTED_MARKER { color: red; }')

    // An apostrophe inside a comment in the array body must not be mistaken
    // for a string delimiter, which would swallow the real entry.
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-commented',
        template: '<p>commented</p>',
        styleUrls: [
          /* don't drop this */
          './ps-commented.component.css',
        ],
      })
      export class CommentedComponent {}
    `
    writeFileSync(commentedPath, source)
    await transformSource(plugin, source, commentedPath)

    writeFileSync(realCssPath, '.PS_COMMENTED_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(realCssPath),
      [{ id: normalizePath(realCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${commentedPath}@CommentedComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_COMMENTED_MARKER')
  })

  it('serves the inline styles of a class whose array carries a comment', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const inlineCommentPath = join(appDir, 'ps-inline-comment.component.ts')
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-inline-comment',
        template: '<p>inline</p>',
        styles: [
          /* it's fine */
          '.PS_INLINE_COMMENT_MARKER { color: red; }',
        ],
      })
      export class InlineCommentComponent {}
    `
    writeFileSync(inlineCommentPath, source)
    await transformSource(plugin, source, inlineCommentPath)

    const edited = source.replace('color: red', 'color: green')
    writeFileSync(inlineCommentPath, edited)
    // Editing the `.ts` itself takes the component-file branch, which keys
    // `componentsByFile` and `pendingHmrUpdates` on `ctx.file` verbatim. Use
    // the same spelling here, for `transform` above, and for the endpoint
    // request below: on Windows a normalized path and a `join()` path differ
    // as strings, and the lookup would miss. Real Vite hands both hooks the
    // same normalized spelling, so only a test can mix them.
    const ctx = createMockHmrContext(inlineCommentPath, [{ id: inlineCommentPath }], mockServer)
    await callHandleHotUpdate(plugin, ctx)

    expectDispatched(mockServer, `${inlineCommentPath}@InlineCommentComponent`)
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${inlineCommentPath}@InlineCommentComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_INLINE_COMMENT_MARKER')
  })
  it('serves no styles to a class that declares none, even beside a styled sibling', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const styledCssPath = join(appDir, 'ps-bare-styled.component.css')
    const barePath = join(appDir, 'ps-bare.component.ts')
    writeFileSync(styledCssPath, '.PS_BARE_SIBLING_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-styled',
        template: '<p>styled</p>',
        styleUrls: ['./ps-bare-styled.component.css'],
      })
      export class StyledComponent {}
      @Component({
        selector: 'app-ps-bare',
        template: '<p>bare</p>',
      })
      export class BareComponent {}
    `
    writeFileSync(barePath, source)
    await transformSource(plugin, source, barePath)

    writeFileSync(styledCssPath, '.PS_BARE_SIBLING_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(styledCssPath),
      [{ id: normalizePath(styledCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    // BareComponent declares no styles at all: it must get NONE, not the
    // file-level union of its sibling's scoped CSS.
    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${barePath}@BareComponent`,
    )
    expect(body).not.toBe('')
    expect(body).not.toContain('PS_BARE_SIBLING_MARKER')
    expect(body).not.toContain('styles:')
  })

  it('serves both inline styles and styleUrls when a class declares both', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const extCssPath = join(appDir, 'ps-merge-ext.component.css')
    const mergePath = join(appDir, 'ps-merge.component.ts')
    writeFileSync(extCssPath, '.PS_MERGE_EXTERNAL_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-merge',
        template: '<p>merge</p>',
        styles: ['.PS_MERGE_INLINE_MARKER { color: blue; }'],
        styleUrls: ['./ps-merge-ext.component.css'],
      })
      export class MergeComponent {}
    `
    writeFileSync(mergePath, source)
    await transformSource(plugin, source, mergePath)

    writeFileSync(extCssPath, '.PS_MERGE_EXTERNAL_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(extCssPath),
      [{ id: normalizePath(extCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${mergePath}@MergeComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_MERGE_INLINE_MARKER')
    expect(body).toContain('PS_MERGE_EXTERNAL_MARKER')
    // Angular's order: the decorator's own inline `styles` first, then the
    // resolved `styleUrl(s)` content appended (see `resolve_styles`).
    expect(body.indexOf('PS_MERGE_INLINE_MARKER')).toBeLessThan(
      body.indexOf('PS_MERGE_EXTERNAL_MARKER'),
    )
  })

  it('falls back to the file-level styleUrls when a class uses a non-literal entry', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const constCssPath = join(appDir, 'ps-const.component.css')
    const constPath = join(appDir, 'ps-const.component.ts')
    writeFileSync(constCssPath, '.PS_CONST_MARKER { color: red; }')

    // The URL comes from a const, so the per-class locator finds the field but
    // reads no string literal out of it. That must fall back to the file-level
    // list (which the Rust extractor DOES fold), not serve an empty style set.
    const source = `
      import { Component } from '@angular/core';
      const STYLE_URL = './ps-const.component.css';
      @Component({
        selector: 'app-ps-const',
        template: '<p>const</p>',
        styleUrls: [STYLE_URL],
      })
      export class ConstComponent {}
    `
    writeFileSync(constPath, source)
    await transformSource(plugin, source, constPath)

    writeFileSync(constCssPath, '.PS_CONST_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(constCssPath),
      [{ id: normalizePath(constCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${constPath}@ConstComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_CONST_MARKER')
  })

  it('falls back when the singular `styleUrl` is a same-file constant', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const ownCssPath = join(appDir, 'ps-singconst-own.component.css')
    const sibCssPath = join(appDir, 'ps-singconst-sib.component.css')
    const singConstPath = join(appDir, 'ps-singconst.component.ts')
    writeFileSync(ownCssPath, '.PS_SINGCONST_OWN_MARKER { color: red; }')
    writeFileSync(sibCssPath, '.PS_SINGCONST_SIB_MARKER { color: red; }')

    // The Rust extractor folds the const (verified: it reports ./own.css),
    // but the text scan cannot. That is "unknown", not "no styles" — serving
    // an empty set would strip this component's CSS entirely.
    const source = `
      import { Component } from '@angular/core';
      const STYLE_URL = './ps-singconst-own.component.css';
      @Component({
        selector: 'app-ps-singconst',
        template: '<p>const</p>',
        styleUrl: STYLE_URL,
      })
      export class SingConstComponent {}
      @Component({
        selector: 'app-ps-singconst-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-singconst-sib.component.css'],
      })
      export class SingConstSiblingComponent {}
    `
    writeFileSync(singConstPath, source)
    await transformSource(plugin, source, singConstPath)

    writeFileSync(ownCssPath, '.PS_SINGCONST_OWN_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(ownCssPath),
      [{ id: normalizePath(ownCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${singConstPath}@SingConstComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_SINGCONST_OWN_MARKER')
  })

  it('serves no styles for an explicitly empty `styleUrls` array', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const sibCssPath = join(appDir, 'ps-emptyurls-sib.component.css')
    const emptyUrlsPath = join(appDir, 'ps-emptyurls.component.ts')
    writeFileSync(sibCssPath, '.PS_EMPTYURLS_SIB_MARKER { color: red; }')

    // `styleUrls: []` is valid and definitive: this class has no styles. It
    // must not inherit the file-level union.
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-emptyurls',
        template: '<p>empty</p>',
        styleUrls: [],
      })
      export class EmptyUrlsComponent {}
      @Component({
        selector: 'app-ps-emptyurls-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-emptyurls-sib.component.css'],
      })
      export class EmptyUrlsSiblingComponent {}
    `
    writeFileSync(emptyUrlsPath, source)
    await transformSource(plugin, source, emptyUrlsPath)

    writeFileSync(sibCssPath, '.PS_EMPTYURLS_SIB_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(sibCssPath),
      [{ id: normalizePath(sibCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${emptyUrlsPath}@EmptyUrlsComponent`,
    )
    expect(body).not.toBe('')
    expect(body).not.toContain('PS_EMPTYURLS_SIB_MARKER')
  })

  it('serves no styles for an explicitly empty inline `styles` array', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const sibCssPath = join(appDir, 'ps-emptyinline-sib.component.css')
    const emptyInlinePath = join(appDir, 'ps-emptyinline.component.ts')
    writeFileSync(sibCssPath, '.PS_EMPTYINLINE_SIB_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-emptyinline',
        template: '<p>empty</p>',
        styles: [],
      })
      export class EmptyInlineComponent {}
      @Component({
        selector: 'app-ps-emptyinline-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-emptyinline-sib.component.css'],
      })
      export class EmptyInlineSiblingComponent {}
    `
    writeFileSync(emptyInlinePath, source)
    await transformSource(plugin, source, emptyInlinePath)

    writeFileSync(sibCssPath, '.PS_EMPTYINLINE_SIB_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(sibCssPath),
      [{ id: normalizePath(sibCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${emptyInlinePath}@EmptyInlineComponent`,
    )
    expect(body).not.toBe('')
    expect(body).not.toContain('PS_EMPTYINLINE_SIB_MARKER')
  })

  it('falls back when a `styleUrls` array mixes a constant with a literal', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const constCssPath = join(appDir, 'ps-mixed-const.component.css')
    const litCssPath = join(appDir, 'ps-mixed-lit.component.css')
    const mixedPath = join(appDir, 'ps-mixed.component.ts')
    writeFileSync(constCssPath, '.PS_MIXED_CONST_MARKER { color: red; }')
    writeFileSync(litCssPath, '.PS_MIXED_LIT_MARKER { color: red; }')

    // One entry is a const the text scan cannot read. Returning just the
    // literal would silently drop a stylesheet — the array is unknown, not
    // partially known.
    const source = `
      import { Component } from '@angular/core';
      const MIXED_STYLE = './ps-mixed-const.component.css';
      @Component({
        selector: 'app-ps-mixed',
        template: '<p>mixed</p>',
        styleUrls: [MIXED_STYLE, './ps-mixed-lit.component.css'],
      })
      export class MixedComponent {}
    `
    writeFileSync(mixedPath, source)
    await transformSource(plugin, source, mixedPath)

    writeFileSync(litCssPath, '.PS_MIXED_LIT_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(litCssPath),
      [{ id: normalizePath(litCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${mixedPath}@MixedComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_MIXED_LIT_MARKER')
    expect(body).toContain('PS_MIXED_CONST_MARKER')
  })

  it('falls back when a `styleUrl` template literal is interpolated', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const interpCssPath = join(appDir, 'ps-interp.component.css')
    const interpPath = join(appDir, 'ps-interp.component.ts')
    writeFileSync(interpCssPath, '.PS_INTERP_MARKER { color: red; }')

    // The Rust extractor folds this to ./ps-interp.component.css; the raw
    // slice `${DIR}/ps-interp.component.css` is not a real path.
    const source = `
      import { Component } from '@angular/core';
      const DIR = '.';
      @Component({
        selector: 'app-ps-interp',
        template: '<p>interp</p>',
        styleUrl: \`\${DIR}/ps-interp.component.css\`,
      })
      export class InterpComponent {}
    `
    writeFileSync(interpPath, source)
    await transformSource(plugin, source, interpPath)

    writeFileSync(interpCssPath, '.PS_INTERP_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(interpCssPath),
      [{ id: normalizePath(interpCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${interpPath}@InterpComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_INTERP_MARKER')
  })

  // Quoted metadata keys are valid TS, and the Rust extractor resolves
  // them (verified: `'styleUrls'` reports its URL). Reading the key as
  // absent makes this class look styleless, which serves it nothing.
  it('serves the own styleUrls of a class whose key is single-quoted', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const quotedCssPath = join(appDir, 'ps-quoted-own.component.css')
    const quotedSibCssPath = join(appDir, 'ps-quoted-sib.component.css')
    const quotedPath = join(appDir, 'ps-quoted.component.ts')
    writeFileSync(quotedCssPath, '.PS_QUOTED_OWN_MARKER { color: red; }')
    writeFileSync(quotedSibCssPath, '.PS_QUOTED_SIB_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-quoted',
        template: '<p>quoted</p>',
        'styleUrls': ['./ps-quoted-own.component.css'],
      })
      export class QuotedKeyComponent {}
      @Component({
        selector: 'app-ps-quoted-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-quoted-sib.component.css'],
      })
      export class QuotedKeySiblingComponent {}
    `
    writeFileSync(quotedPath, source)
    await transformSource(plugin, source, quotedPath)

    writeFileSync(quotedCssPath, '.PS_QUOTED_OWN_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(quotedCssPath),
      [{ id: normalizePath(quotedCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${quotedPath}@QuotedKeyComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_QUOTED_OWN_MARKER')
    expect(body).not.toContain('PS_QUOTED_SIB_MARKER')
  })

  it('serves the own styleUrls of a class whose key is double-quoted', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const dqCssPath = join(appDir, 'ps-dquoted-own.component.css')
    const dqSibCssPath = join(appDir, 'ps-dquoted-sib.component.css')
    const dqPath = join(appDir, 'ps-dquoted.component.ts')
    writeFileSync(dqCssPath, '.PS_DQUOTED_OWN_MARKER { color: red; }')
    writeFileSync(dqSibCssPath, '.PS_DQUOTED_SIB_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-dquoted',
        template: '<p>dquoted</p>',
        "styleUrls": ['./ps-dquoted-own.component.css'],
      })
      export class DQuotedKeyComponent {}
      @Component({
        selector: 'app-ps-dquoted-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-dquoted-sib.component.css'],
      })
      export class DQuotedKeySiblingComponent {}
    `
    writeFileSync(dqPath, source)
    await transformSource(plugin, source, dqPath)

    writeFileSync(dqCssPath, '.PS_DQUOTED_OWN_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(dqCssPath),
      [{ id: normalizePath(dqCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${dqPath}@DQuotedKeyComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_DQUOTED_OWN_MARKER')
    expect(body).not.toContain('PS_DQUOTED_SIB_MARKER')
  })

  // A computed key hides the field from this scan while the Rust extractor
  // still resolves it, so the class must fall back rather than be served
  // nothing — the file-level list carries its stylesheet.
  it('falls back when a style field is declared under a computed key', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const computedCssPath = join(appDir, 'ps-computed-own.component.css')
    const computedPath = join(appDir, 'ps-computed.component.ts')
    writeFileSync(computedCssPath, '.PS_COMPUTED_OWN_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      const K = 'styleUrls';
      @Component({
        selector: 'app-ps-computed',
        template: '<p>computed</p>',
        [K]: ['./ps-computed-own.component.css'],
      })
      export class ComputedKeyComponent {}
    `
    writeFileSync(computedPath, source)
    await transformSource(plugin, source, computedPath)

    writeFileSync(computedCssPath, '.PS_COMPUTED_OWN_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(computedCssPath),
      [{ id: normalizePath(computedCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${computedPath}@ComputedKeyComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_COMPUTED_OWN_MARKER')
  })

  // A spread is the one unreadable form the compiler ALSO drops, so
  // "declares no styles" is what the compiled component actually gets.
  // Falling back here would hand this class its sibling's CSS instead.
  it('serves no styles for a spread-only decorator, matching the compiler', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const spreadSibCssPath = join(appDir, 'ps-spread-sib.component.css')
    const spreadPath = join(appDir, 'ps-spread.component.ts')
    writeFileSync(spreadSibCssPath, '.PS_SPREAD_SIB_MARKER { color: red; }')

    const source = `
      import { Component } from '@angular/core';
      const BASE = { selector: 'app-ps-spread' };
      @Component({
        template: '<p>spread</p>',
        ...BASE,
      })
      export class SpreadComponent {}
      @Component({
        selector: 'app-ps-spread-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-spread-sib.component.css'],
      })
      export class SpreadSiblingComponent {}
    `
    writeFileSync(spreadPath, source)
    await transformSource(plugin, source, spreadPath)

    writeFileSync(spreadSibCssPath, '.PS_SPREAD_SIB_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(spreadSibCssPath),
      [{ id: normalizePath(spreadSibCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${spreadPath}@SpreadComponent`,
    )
    expect(body).not.toBe('')
    expect(body).not.toContain('PS_SPREAD_SIB_MARKER')
  })
  it('serves a styleUrls entry written with a JavaScript escape', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const escCssPath = join(appDir, 'ps-esc.component.css')
    const escSibCssPath = join(appDir, 'ps-esc-sib.component.css')
    const escPath = join(appDir, 'ps-esc.component.ts')
    writeFileSync(escCssPath, '.PS_ESC_OWN_MARKER { color: red; }')
    writeFileSync(escSibCssPath, '.PS_ESC_SIB_MARKER { color: blue; }')

    // `\u002e` is `.` — the cooked value is `./ps-esc.component.css`, which
    // is what the Rust extractor sees. A raw read yields a path that does
    // not exist, and the per-style catch would swallow the failure.
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-esc',
        template: '<p>esc</p>',
        styleUrls: ['\\u002e/ps-esc\\u002ecomponent.css'],
      })
      export class EscComponent {}
      @Component({
        selector: 'app-ps-esc-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-esc-sib.component.css'],
      })
      export class EscSiblingComponent {}
    `
    writeFileSync(escPath, source)
    await transformSource(plugin, source, escPath)

    writeFileSync(escSibCssPath, '.PS_ESC_SIB_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(escSibCssPath),
      [{ id: normalizePath(escSibCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(getMiddleware(mockServer), `${escPath}@EscComponent`)
    expect(body).not.toBe('')
    expect(body).toContain('PS_ESC_OWN_MARKER')
    expect(body).not.toContain('PS_ESC_SIB_MARKER')
  })

  it('ignores a commented-out decorator when picking the styles of a class', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const realCssPath = join(appDir, 'ps-cmt-real.component.css')
    const oldCssPath = join(appDir, 'ps-cmt-old.component.css')
    const cmtSibCssPath = join(appDir, 'ps-cmt-sib.component.css')
    const cmtPath = join(appDir, 'ps-cmt.component.ts')
    writeFileSync(realCssPath, '.PS_CMT_REAL_MARKER { color: red; }')
    writeFileSync(oldCssPath, '.PS_CMT_OLD_MARKER { color: blue; }')
    writeFileSync(cmtSibCssPath, '.PS_CMT_SIB_MARKER { color: teal; }')

    // A leftover commented decorator sits between the real one and the class.
    // Enumeration must not pair the class with the commented occurrence.
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-ps-cmt',
        template: '<p>cmt</p>',
        styleUrls: ['./ps-cmt-real.component.css'],
      })
      // @Component({ styleUrl: './ps-cmt-old.component.css' })
      export class CommentedComponent {}
      @Component({
        selector: 'app-ps-cmt-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-cmt-sib.component.css'],
      })
      export class CommentedSiblingComponent {}
    `
    writeFileSync(cmtPath, source)
    await transformSource(plugin, source, cmtPath)

    writeFileSync(cmtSibCssPath, '.PS_CMT_SIB_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(cmtSibCssPath),
      [{ id: normalizePath(cmtSibCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${cmtPath}@CommentedComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_CMT_REAL_MARKER')
    expect(body).not.toContain('PS_CMT_OLD_MARKER')
    expect(body).not.toContain('PS_CMT_SIB_MARKER')
  })
  it('serves the own stylesheet of a class using a shorthand `styleUrl`', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithRealConfig(plugin)

    const ownCssPath = join(appDir, 'ps-shorthand-own.component.css')
    const sibCssPath = join(appDir, 'ps-shorthand-sib.component.css')
    const shorthandPath = join(appDir, 'ps-shorthand.component.ts')
    writeFileSync(ownCssPath, '.PS_SHORTHAND_OWN_MARKER { color: red; }')
    writeFileSync(sibCssPath, '.PS_SHORTHAND_SIB_MARKER { color: red; }')

    // The shorthand key is a style field this scan cannot resolve, but the
    // Rust extractor folds the constant behind it. Reading it as "declares
    // no styles" would strip the component's CSS.
    const source = `
      import { Component } from '@angular/core';
      const styleUrl = './ps-shorthand-own.component.css';
      @Component({
        selector: 'app-ps-shorthand',
        template: '<p>shorthand</p>',
        styleUrl,
      })
      export class ShorthandComponent {}
      @Component({
        selector: 'app-ps-shorthand-sib',
        template: '<p>sib</p>',
        styleUrls: ['./ps-shorthand-sib.component.css'],
      })
      export class ShorthandSiblingComponent {}
    `
    writeFileSync(shorthandPath, source)
    await transformSource(plugin, source, shorthandPath)

    writeFileSync(ownCssPath, '.PS_SHORTHAND_OWN_MARKER { color: green; }')
    const ctx = createMockHmrContext(
      normalizePath(ownCssPath),
      [{ id: normalizePath(ownCssPath) }],
      mockServer,
    )
    await callHandleHotUpdate(plugin, ctx)
    expectDispatched(mockServer, `${shorthandPath}@ShorthandComponent`)

    const body = await invokeAngularMiddleware(
      getMiddleware(mockServer),
      `${shorthandPath}@ShorthandComponent`,
    )
    expect(body).not.toBe('')
    expect(body).toContain('PS_SHORTHAND_OWN_MARKER')
    // The sibling's stylesheet rides along, because this is the file-level
    // fallback: it is the union for the file. That is the known limitation
    // tracked in #456, asserted the same way as the singular-constant case
    // above. The defect fixed here is the own stylesheet going MISSING.
  })
})

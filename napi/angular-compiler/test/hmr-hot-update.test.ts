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

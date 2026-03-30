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
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
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

beforeAll(() => {
  tempDir = mkdtempSync(join(tmpdir(), 'hmr-test-'))
  appDir = join(tempDir, 'src', 'app')
  mkdirSync(appDir, { recursive: true })

  templatePath = join(appDir, 'app.component.html')
  stylePath = join(appDir, 'app.component.css')

  writeFileSync(templatePath, '<h1>Hello</h1>')
  writeFileSync(stylePath, 'h1 { color: red; }')
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

  return mockServer
}

/**
 * Transform a component that references external template + style files,
 * populating resourceToComponent and componentIds.
 */
async function transformComponent(plugin: Plugin) {
  const componentFile = join(appDir, 'app.component.ts')
  const componentSource = `
    import { Component } from '@angular/core';

    @Component({
      selector: 'app-root',
      templateUrl: './app.component.html',
      styleUrls: ['./app.component.css'],
    })
    export class AppComponent {}
  `

  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  await plugin.transform.handler.call(
    { error() {}, warn() {} } as any,
    componentSource,
    componentFile,
  )
}

describe('handleHotUpdate - Issue #185', () => {
  it('should let non-component CSS files pass through to Vite HMR', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    // A global CSS file (not referenced by any component's styleUrls)
    const globalCssFile = normalizePath(join(tempDir, 'src', 'styles.css'))
    const mockModules = [{ id: globalCssFile, type: 'css' }]
    const ctx = createMockHmrContext(globalCssFile, mockModules)

    let result: ModuleNode[] | void | undefined
    if (typeof plugin.handleHotUpdate === 'function') {
      result = await plugin.handleHotUpdate(ctx)
    }

    // Non-component CSS should NOT be swallowed — either undefined (pass through)
    // or the original modules array, but NOT an empty array
    if (result !== undefined) {
      expect(result).toEqual(mockModules)
    }
  })

  it('should return [] for component CSS files managed by custom watcher', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // The component's CSS file IS in resourceToComponent
    const componentCssFile = normalizePath(stylePath)
    const mockModules = [{ id: componentCssFile }]
    const ctx = createMockHmrContext(componentCssFile, mockModules, mockServer)

    let result: ModuleNode[] | void | undefined
    if (typeof plugin.handleHotUpdate === 'function') {
      result = await plugin.handleHotUpdate(ctx)
    }

    // Component resources MUST be swallowed (return [])
    expect(result).toEqual([])
  })

  it('should return [] for component template HTML files managed by custom watcher', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)
    await transformComponent(plugin)

    // The component's HTML template IS in resourceToComponent
    const componentHtmlFile = normalizePath(templatePath)
    const ctx = createMockHmrContext(componentHtmlFile, [{ id: componentHtmlFile }], mockServer)

    let result: ModuleNode[] | void | undefined
    if (typeof plugin.handleHotUpdate === 'function') {
      result = await plugin.handleHotUpdate(ctx)
    }

    // Component templates MUST be swallowed (return [])
    expect(result).toEqual([])
  })

  it('should not swallow non-resource HTML files', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    // index.html is NOT a component template
    const indexHtml = normalizePath(join(tempDir, 'index.html'))
    const mockModules = [{ id: indexHtml }]
    const ctx = createMockHmrContext(indexHtml, mockModules)

    let result: ModuleNode[] | void | undefined
    if (typeof plugin.handleHotUpdate === 'function') {
      result = await plugin.handleHotUpdate(ctx)
    }

    // Non-component HTML should pass through, not be swallowed
    if (result !== undefined) {
      expect(result).toEqual(mockModules)
    }
  })

  it('should pass through non-style/template files unchanged', async () => {
    const plugin = getAngularPlugin()
    await setupPluginWithServer(plugin)

    const utilFile = normalizePath(join(tempDir, 'src', 'utils.ts'))
    const mockModules = [{ id: utilFile }]
    const ctx = createMockHmrContext(utilFile, mockModules)

    let result: ModuleNode[] | void | undefined
    if (typeof plugin.handleHotUpdate === 'function') {
      result = await plugin.handleHotUpdate(ctx)
    }

    // Non-Angular .ts files should pass through with their modules
    if (result !== undefined) {
      expect(result).toEqual(mockModules)
    }
  })
})

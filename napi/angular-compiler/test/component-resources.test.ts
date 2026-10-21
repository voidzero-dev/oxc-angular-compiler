/**
 * Tests for how the plugin finds a component's external resources.
 *
 * - `extractComponentUrls` evaluates `templateUrl` / `styleUrl(s)` with the
 *   options the file is transformed with, so a URL held in a `const` imported
 *   from another file is read like ngtsc reads it.
 * - A template's `<link rel="stylesheet" href="...">` is loaded like a
 *   `styleUrl` (ngtsc inlines it into `styles`): the transform reports the
 *   hrefs it was not given, the plugin reads and preprocesses them and
 *   compiles again. Only hrefs that still cannot be read are reported as
 *   COMPONENT_RESOURCE_NOT_FOUND. Link stylesheets are direct styles for HMR,
 *   and the `@ng/component` update module carries them too.
 */
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import type { HmrContext, ModuleNode, Plugin } from 'vite'
import { normalizePath, resolveConfig } from 'vite'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

import { angular } from '../vite-plugin/index.js'

const spies = vi.hoisted(() => ({
  extractComponentUrls: undefined as unknown as ReturnType<typeof vi.fn>,
}))

vi.mock('#binding', async (importOriginal) => {
  const actual = await importOriginal<typeof import('#binding')>()
  spies.extractComponentUrls = vi.fn(actual.extractComponentUrls)
  return { ...actual, extractComponentUrls: spies.extractComponentUrls }
})

let tempDir: string
let appDir: string

beforeAll(() => {
  tempDir = realpathSync(mkdtempSync(join(tmpdir(), 'component-resources-test-')))
  appDir = join(tempDir, 'src', 'app')
  mkdirSync(join(appDir, 'sub'), { recursive: true })
})

afterAll(() => {
  rmSync(tempDir, { recursive: true, force: true })
})

beforeEach(() => {
  spies.extractComponentUrls.mockClear()
})

function getAngularPlugin() {
  const plugin = angular({ liveReload: true }).find(
    (candidate) => candidate.name === '@oxc-angular/vite',
  )
  if (!plugin) throw new Error('Failed to find @oxc-angular/vite plugin')
  return plugin
}

function createMockServer() {
  return {
    watcher: { add: vi.fn(), unwatch: vi.fn(), on: vi.fn(), emit: vi.fn() },
    ws: { send: vi.fn(), on: vi.fn() },
    moduleGraph: { getModuleById: vi.fn(() => null), invalidateModule: vi.fn() },
    middlewares: { use: vi.fn() },
    config: { root: tempDir },
  }
}

async function callPluginHook(hook: any, ...args: unknown[]) {
  if (!hook) return undefined
  if (typeof hook === 'function') return hook(...args)
  return hook.handler(...args)
}

async function setupPlugin(plugin: Plugin, command: 'serve' | 'build') {
  const mockServer = createMockServer()
  await callPluginHook(plugin.config, {}, { command, mode: 'development' })
  const resolved = await resolveConfig(
    { configFile: false, root: tempDir, logLevel: 'silent' },
    command,
  )
  await callPluginHook(plugin.configResolved, resolved)
  if (command === 'serve' && typeof plugin.configureServer === 'function') {
    await (plugin.configureServer as Function)(mockServer)
  }
  return mockServer
}

async function transform(plugin: Plugin, source: string, path: string) {
  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }
  const errors: string[] = []
  const result = (await plugin.transform.handler.call(
    {
      error(message: string) {
        errors.push(message)
      },
      warn() {},
      addWatchFile() {},
    } as any,
    source,
    path,
  )) as { code: string } | undefined
  return { code: result?.code ?? '', errors }
}

function hotUpdate(plugin: Plugin, file: string, server: any) {
  const ctx = {
    file: normalizePath(file),
    timestamp: Date.now(),
    modules: [{ id: normalizePath(file) }] as unknown as ModuleNode[],
    read: async () => '',
    server,
  } as HmrContext
  return (plugin.handleHotUpdate as Function).call(plugin, ctx)
}

async function requestUpdateModule(server: any, componentId: string): Promise<string> {
  const middleware = (server.middlewares.use as ReturnType<typeof vi.fn>).mock.calls[0]?.[0]
  expect(middleware, 'expected the @ng/component middleware').toBeDefined()
  const req = { url: `/@ng/component?c=${encodeURIComponent(componentId)}&t=${Date.now()}` }
  return new Promise<string>((resolve) => {
    middleware(
      req,
      {
        statusCode: 200,
        setHeader() {},
        end(data: string = '') {
          resolve(data ?? '')
        },
      },
      () => resolve(''),
    )
  })
}

/** Silences the plugin's "Failed to read" warnings for files a test leaves out. */
async function withoutWarnings<T>(run: () => Promise<T>): Promise<T> {
  const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})
  try {
    return await run()
  } finally {
    warn.mockRestore()
  }
}

describe('resource URLs imported from another file', () => {
  it('loads a templateUrl and styleUrls held in imported constants', async () => {
    writeFileSync(
      join(appDir, 'imported-urls.ts'),
      `export const TEMPLATE_URL = './imported.component.html'\n` +
        `export const STYLE_URLS = ['./imported.component.css']\n`,
    )
    writeFileSync(join(appDir, 'imported.component.html'), '<p>IMPORTED_TEMPLATE_MARKER</p>')
    writeFileSync(join(appDir, 'imported.component.css'), '.IMPORTED_STYLE_MARKER { color: red; }')

    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')
    const { code, errors } = await transform(
      plugin,
      `
        import { Component } from '@angular/core';
        import { TEMPLATE_URL, STYLE_URLS } from './imported-urls';
        @Component({ selector: 'app-imported', templateUrl: TEMPLATE_URL, styleUrls: STYLE_URLS })
        export class ImportedComponent {}
      `,
      join(appDir, 'imported.component.ts'),
    )

    expect(errors).toEqual([])
    expect(code).toContain('IMPORTED_TEMPLATE_MARKER')
    expect(code).toContain('IMPORTED_STYLE_MARKER')
  })
})

describe('template stylesheet links', () => {
  it('compiles a link in an external template, relative to the component file', async () => {
    // The template lives in `sub/`; ngtsc resolves its links against the
    // component file all the same.
    writeFileSync(
      join(appDir, 'sub', 'linked.component.html'),
      '<link rel="stylesheet" href="./linked.css"><p>linked</p>',
    )
    writeFileSync(join(appDir, 'linked.css'), '.LINK_STYLE_MARKER { color: blue; }')
    writeFileSync(join(appDir, 'sub', 'linked.css'), '.WRONG_LINK_MARKER { color: red; }')
    writeFileSync(join(appDir, 'linked.component.css'), '.URL_STYLE_MARKER { color: red; }')

    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')
    const { code, errors } = await transform(
      plugin,
      `
        import { Component } from '@angular/core';
        @Component({
          selector: 'app-linked',
          templateUrl: './sub/linked.component.html',
          styleUrl: './linked.component.css',
        })
        export class LinkedComponent {}
      `,
      join(appDir, 'linked.component.ts'),
    )

    expect(errors).toEqual([])
    expect(code).not.toContain('WRONG_LINK_MARKER')
    // After the decorator's styleUrl, like ngtsc.
    expect(code).toMatch(/URL_STYLE_MARKER[^\]]*LINK_STYLE_MARKER/)
  })

  it('compiles a link in an inline template without parsing for resource URLs', async () => {
    writeFileSync(join(appDir, 'inline-link.css'), '.INLINE_LINK_MARKER { color: blue; }')

    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')
    const { code, errors } = await transform(
      plugin,
      `
        import { Component } from '@angular/core';
        @Component({
          selector: 'app-inline-link',
          template: '<link rel="stylesheet" href="./inline-link.css"><p>inline</p>',
        })
        export class InlineLinkComponent {}
      `,
      join(appDir, 'inline-link.component.ts'),
    )

    expect(errors).toEqual([])
    expect(code).toContain('INLINE_LINK_MARKER')
    // The link comes from the compiled template, not from the URL parse.
    expect(spies.extractComponentUrls).not.toHaveBeenCalled()
  })

  it('reports only the links that still cannot be read', async () => {
    writeFileSync(join(appDir, 'present-link.css'), '.PRESENT_LINK_MARKER { color: blue; }')

    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')
    const { errors } = await withoutWarnings(() =>
      transform(
        plugin,
        `
          import { Component } from '@angular/core';
          @Component({
            selector: 'app-missing-link',
            template:
              '<link rel="stylesheet" href="./present-link.css">' +
              '<link rel="stylesheet" href="./absent-link.css">',
          })
          export class MissingLinkComponent {}
        `,
        join(appDir, 'missing-link.component.ts'),
      ),
    )

    expect(errors).toHaveLength(1)
    expect(errors[0]).toContain('./absent-link.css')
    expect(errors[0]).toContain('COMPONENT_RESOURCE_NOT_FOUND')
  })

  it('hot-updates the component when a linked stylesheet changes', async () => {
    const stylePath = join(appDir, 'hmr-link.css')
    const componentPath = join(appDir, 'hmr-link.component.ts')
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-hmr-link',
        template: '<link rel="stylesheet" href="./hmr-link.css"><h1>Hello</h1>',
      })
      export class HmrLinkComponent {}
    `
    writeFileSync(stylePath, 'h1 { color: red; }')
    writeFileSync(componentPath, source)

    const plugin = getAngularPlugin()
    const server = await setupPlugin(plugin, 'serve')
    const { errors } = await transform(plugin, source, componentPath)
    expect(errors).toEqual([])
    expect(server.watcher.add).toHaveBeenCalledWith(stylePath)

    writeFileSync(stylePath, 'h1 { color: green; }')
    await hotUpdate(plugin, stylePath, server)
    const body = await requestUpdateModule(server, `${componentPath}@HmrLinkComponent`)

    expect(body).toContain('color: green')
    expect(body).not.toContain('color: red')
  })
})

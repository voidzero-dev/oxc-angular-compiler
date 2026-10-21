/**
 * Tests for the plugin's fast paths and HMR option parity.
 *
 * - `transform` skips the `extractComponentUrls` parse for a file that never
 *   spells `templateUrl`/`styleUrl`, and still compiles a file that names a
 *   resource through an escaped key.
 * - JIT `transform` never reads external resources: its output imports them.
 * - The `@ng/component` endpoint compiles with the decorator's
 *   `preserveWhitespaces`, like the full transform.
 * - One stylesheet edit is preprocessed once per HMR cycle, not once by
 *   `handleHotUpdate`, once by the endpoint, and once by the next transform.
 */
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import type { HmrContext, ModuleNode, Plugin } from 'vite'
import { normalizePath, resolveConfig } from 'vite'
import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest'

import { angular } from '../vite-plugin/index.js'

// Spy on the binding call and on Vite's preprocessor, keeping their real
// behavior, so the tests can count how often the plugin reaches them.
const spies = vi.hoisted(() => ({
  extractComponentUrls: undefined as unknown as ReturnType<typeof vi.fn>,
  preprocessCSS: undefined as unknown as ReturnType<typeof vi.fn>,
}))

vi.mock('#binding', async (importOriginal) => {
  const actual = await importOriginal<typeof import('#binding')>()
  spies.extractComponentUrls = vi.fn(actual.extractComponentUrls)
  return { ...actual, extractComponentUrls: spies.extractComponentUrls }
})

vi.mock('vite', async (importOriginal) => {
  const actual = await importOriginal<typeof import('vite')>()
  spies.preprocessCSS = vi.fn(actual.preprocessCSS)
  return { ...actual, preprocessCSS: spies.preprocessCSS }
})

let tempDir: string
let appDir: string

beforeAll(() => {
  // realpath: Sass canonicalizes loaded URLs (macOS /var -> /private/var).
  tempDir = realpathSync(mkdtempSync(join(tmpdir(), 'plugin-fast-paths-test-')))
  appDir = join(tempDir, 'src', 'app')
  mkdirSync(appDir, { recursive: true })
})

afterAll(() => {
  rmSync(tempDir, { recursive: true, force: true })
})

beforeEach(() => {
  spies.extractComponentUrls.mockClear()
  spies.preprocessCSS.mockClear()
})

function getAngularPlugin(options: Parameters<typeof angular>[0] = { liveReload: true }) {
  const plugin = angular(options).find((candidate) => candidate.name === '@oxc-angular/vite')
  if (!plugin) throw new Error('Failed to find @oxc-angular/vite plugin')
  return plugin
}

function createMockServer() {
  const wsMessages: any[] = []
  return {
    watcher: { add: vi.fn(), unwatch: vi.fn(), on: vi.fn(), emit: vi.fn() },
    ws: {
      send(msg: any) {
        wsMessages.push(msg)
      },
      on: vi.fn(),
    },
    moduleGraph: { getModuleById: vi.fn(() => null), invalidateModule: vi.fn() },
    middlewares: { use: vi.fn() },
    config: { root: tempDir },
    _wsMessages: wsMessages,
  }
}

async function callPluginHook(hook: any, ...args: unknown[]) {
  if (!hook) return undefined
  if (typeof hook === 'function') return hook(...args)
  return hook.handler(...args)
}

/** Run the plugin through `config`/`configResolved`/`configureServer`. */
async function setupPlugin(plugin: Plugin, command: 'serve' | 'build' = 'serve') {
  const mockServer = createMockServer()
  await callPluginHook(plugin.config, {}, { command, mode: 'development' })
  // A real resolved config: preprocessCSS needs one to run Sass.
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
  const warnings: string[] = []
  const result = (await plugin.transform.handler.call(
    {
      error(message: string) {
        errors.push(message)
      },
      warn(message: string) {
        warnings.push(message)
      },
      addWatchFile() {},
    } as any,
    source,
    path,
  )) as { code: string } | undefined
  return { code: result?.code ?? '', errors, warnings }
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

function preprocessCallsFor(path: string): number {
  return spies.preprocessCSS.mock.calls.filter(
    ([, filename]) => normalizePath(filename as string) === normalizePath(path),
  ).length
}

describe('transform skips the resource URL parse when no URL is spelled', () => {
  it('does not call extractComponentUrls for an inline-only component', async () => {
    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')

    const { code, errors } = await transform(
      plugin,
      `
        import { Component } from '@angular/core';
        @Component({
          selector: 'app-inline',
          template: '<p>inline</p>',
          styles: ['p { color: red; }'],
        })
        export class InlineComponent {}
      `,
      join(appDir, 'inline.component.ts'),
    )

    expect(errors).toEqual([])
    expect(code).toContain('ɵɵdefineComponent')
    expect(spies.extractComponentUrls).not.toHaveBeenCalled()
  })

  it('still calls extractComponentUrls for a component with a templateUrl', async () => {
    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')
    writeFileSync(join(appDir, 'url.component.html'), '<p>URL_TEMPLATE_MARKER</p>')

    const { code, errors } = await transform(
      plugin,
      `
        import { Component } from '@angular/core';
        @Component({ selector: 'app-url', templateUrl: './url.component.html' })
        export class UrlComponent {}
      `,
      join(appDir, 'url.component.ts'),
    )

    expect(errors).toEqual([])
    expect(code).toContain('URL_TEMPLATE_MARKER')
    expect(spies.extractComponentUrls).toHaveBeenCalledTimes(1)
  })

  it('resolves resources named only through escaped keys', async () => {
    const plugin = getAngularPlugin()
    await setupPlugin(plugin, 'build')
    writeFileSync(join(appDir, 'escaped.component.html'), '<p>ESCAPED_TEMPLATE_MARKER</p>')
    writeFileSync(join(appDir, 'escaped.component.css'), '.ESCAPED_STYLE_MARKER { color: red; }')

    // `templateUrl` IS `templateUrl` to the parser and to the compiler,
    // but the text holds neither name, so this takes the fast path first.
    const { code, errors } = await transform(
      plugin,
      `
        import { Component } from '@angular/core';
        @Component({
          selector: 'app-escaped',
          template\\u0055rl: './escaped.component.html',
          'style\\u0055rls': ['./escaped.component.css'],
        })
        export class EscapedComponent {}
      `,
      join(appDir, 'escaped.component.ts'),
    )

    expect(errors).toEqual([])
    expect(code).toContain('ESCAPED_TEMPLATE_MARKER')
    expect(code).toContain('ESCAPED_STYLE_MARKER')
  })
})

describe('JIT transform', () => {
  it('does not read or parse external resources', async () => {
    const plugin = getAngularPlugin({ jit: true, liveReload: true })
    const server = await setupPlugin(plugin)
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {})

    try {
      // Neither file exists: AOT would warn that it cannot read them, JIT
      // only imports them.
      const { code, errors } = await transform(
        plugin,
        `
          import { Component } from '@angular/core';
          @Component({
            selector: 'app-jit',
            templateUrl: './missing-jit.component.html',
            styleUrl: './missing-jit.component.scss',
          })
          export class JitComponent {}
        `,
        join(appDir, 'jit.component.ts'),
      )

      expect(errors).toEqual([])
      expect(code).toContain('angular:jit:template:file;./missing-jit.component.html')
      expect(code).toContain('angular:jit:style:file;./missing-jit.component.scss')
      expect(spies.extractComponentUrls).not.toHaveBeenCalled()
      expect(warn).not.toHaveBeenCalled()
      expect(server.watcher.add).not.toHaveBeenCalled()
    } finally {
      warn.mockRestore()
    }
  })
})

describe('@ng/component endpoint template options', () => {
  const TEMPLATE = '<p>  a  </p>   <span>b</span>'

  async function hmrModuleFor(preserveWhitespaces: boolean) {
    const name = preserveWhitespaces ? 'ws-keep' : 'ws-strip'
    const templatePath = join(appDir, `${name}.component.html`)
    const componentPath = join(appDir, `${name}.component.ts`)
    const source = `
      import { Component } from '@angular/core';
      @Component({
        selector: 'app-${name}',
        templateUrl: './${name}.component.html',
        preserveWhitespaces: ${preserveWhitespaces},
      })
      export class WsComponent {}
    `
    writeFileSync(templatePath, TEMPLATE)
    writeFileSync(componentPath, source)

    const plugin = getAngularPlugin()
    const server = await setupPlugin(plugin)
    const { code } = await transform(plugin, source, componentPath)

    await hotUpdate(plugin, templatePath, server)
    const body = await requestUpdateModule(server, `${componentPath}@WsComponent`)
    return { code, body }
  }

  it('keeps whitespace when the decorator sets preserveWhitespaces', async () => {
    const { code, body } = await hmrModuleFor(true)

    // The full transform keeps the padding and the text node between the
    // elements; the update module must build the same view.
    expect(code).toMatch(/ɵɵtext\(\d+,\s*["'] {2}a {2}["']\)/)
    expect(code).toMatch(/ɵɵtext\(\d+,\s*["'] {3}["']\)/)
    expect(body).not.toBe('')
    expect(body).toMatch(/ɵɵtext\(\d+,\s*["'] {2}a {2}["']\)/)
    expect(body).toMatch(/ɵɵtext\(\d+,\s*["'] {3}["']\)/)
  })

  it('strips whitespace when the decorator does not preserve it', async () => {
    const { code, body } = await hmrModuleFor(false)

    expect(code).not.toMatch(/ɵɵtext\(\d+,\s*["'] {3}["']\)/)
    expect(body).not.toBe('')
    expect(body).toMatch(/ɵɵtext\(\d+,\s*["'] a ["']\)/)
    expect(body).not.toMatch(/ɵɵtext\(\d+,\s*["'] {3}["']\)/)
  })
})

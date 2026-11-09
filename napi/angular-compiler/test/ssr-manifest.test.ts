import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { afterAll, describe, it, expect } from 'vitest'
/**
 * Tests for SSR manifest generation.
 *
 * These tests verify that the Vite plugin correctly generates the Angular SSR
 * manifests required by AngularNodeAppEngine. Without these manifests, SSR fails with:
 *   "Angular app engine manifest is not set."
 *
 * See: https://github.com/voidzero-dev/oxc-angular-compiler/issues/60
 */

// Import the SSR manifest plugin directly
import {
  ssrManifestPlugin,
  generateAppManifestCode,
  generateAppEngineManifestCode,
} from '../vite-plugin/angular-ssr-manifest-plugin.js'

describe('SSR Manifest Generation (Issue #60)', () => {
  describe('generateAppManifestCode', () => {
    it('should generate valid app manifest code with bootstrap import', () => {
      const code = generateAppManifestCode({
        ssrEntryImport: './src/main.server',
        baseHref: '/',
        indexHtmlContent: '<html><body><app-root></app-root></body></html>',
      })

      expect(code).toContain('ɵsetAngularAppManifest')
      expect(code).toContain('./src/main.server')
      expect(code).toContain('bootstrap')
      expect(code).toContain('inlineCriticalCss')
      expect(code).toContain('index.server.html')
      expect(code).toContain('<html><body><app-root></app-root></body></html>')
    })

    it('should escape template literal characters in HTML', () => {
      const code = generateAppManifestCode({
        ssrEntryImport: './src/main.server',
        baseHref: '/',
        indexHtmlContent: '<html><body>${unsafe}`backtick`\\backslash</body></html>',
      })

      // Template literal chars should be escaped
      expect(code).toContain('\\${unsafe}')
      expect(code).toContain('\\`backtick\\`')
      expect(code).toContain('\\\\backslash')
      // The dollar sign should be escaped to prevent template literal injection
      expect(code).not.toMatch(/[^\\]\$\{unsafe\}/)
    })

    it('should use custom baseHref', () => {
      const code = generateAppManifestCode({
        ssrEntryImport: './src/main.server',
        baseHref: '/my-app/',
        indexHtmlContent: '<html></html>',
      })

      expect(code).toContain("baseHref: '/my-app/'")
    })
  })

  describe('generateAppEngineManifestCode', () => {
    it('should generate valid app engine manifest code', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/',
      })

      expect(code).toContain('ɵsetAngularAppEngineManifest')
      expect(code).toContain("basePath: '/'")
      expect(code).toContain('supportedLocales')
      expect(code).toContain('entryPoints')
      expect(code).toContain('allowedHosts')
    })

    it('should strip trailing slash from basePath (except root)', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/my-app/',
      })

      expect(code).toContain("basePath: '/my-app'")
    })

    it('should keep root basePath as-is', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/',
      })

      expect(code).toContain("basePath: '/'")
    })

    it('should include ɵgetOrCreateAngularServerApp in entry points', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/',
      })

      expect(code).toContain('ɵgetOrCreateAngularServerApp')
      expect(code).toContain('ɵdestroyAngularServerApp')
      expect(code).toContain('ɵextractRoutesAndCreateRouteTree')
    })
  })

  describe('ssrManifestPlugin', () => {
    it('should create a plugin with correct name', () => {
      const plugin = ssrManifestPlugin({})
      expect(plugin.name).toBe('@oxc-angular/vite-ssr-manifest')
    })

    it('should only apply to build mode', () => {
      const plugin = ssrManifestPlugin({})
      expect(plugin.apply).toBe('build')
    })

    it('should inject the manifests only in server environments', () => {
      // One config can build the client and the server environments
      // (`vite build --app`); `vite build --ssr` builds the `ssr` one.
      const plugin = ssrManifestPlugin({})
      ;(plugin.configResolved as Function)({ root: '/app', base: '/' })
      const transform = (consumer: 'client' | 'server') =>
        (plugin.transform as Function).call(
          { environment: { config: { consumer } } },
          'const engine = new AngularNodeAppEngine();',
          '/app/src/server.ts',
        )

      expect(transform('client')).toBeUndefined()
      expect(transform('server')?.code).toContain('ɵsetAngularAppManifest')
    })

    it('should not inject the manifests twice', () => {
      // A server environment can bundle another one's output (Nitro).
      const plugin = ssrManifestPlugin({})
      ;(plugin.configResolved as Function)({ root: '/app', base: '/' })
      const transform = (code: string) =>
        (plugin.transform as Function).call(
          { environment: { config: { consumer: 'server' } } },
          code,
          '/app/.nitro/vite/services/ssr/index.mjs',
        )

      const once = transform('const engine = new AngularNodeAppEngine();').code
      expect(transform(once)).toBeUndefined()
    })

    it('should do nothing outside Vite (plain rolldown has no environment)', async () => {
      const plugin = ssrManifestPlugin({})
      ;(plugin.configResolved as Function)({ root: '/app', base: '/' })
      await (plugin.buildStart as Function).call({})
      expect(
        (plugin.transform as Function).call(
          {},
          'const engine = new AngularNodeAppEngine();',
          '/app/src/server.ts',
        ),
      ).toBeUndefined()
    })
  })

  describe('index.server.html (#696)', () => {
    const SOURCE =
      '<html><body><app-root></app-root><script type="module" src="/src/main.ts"></script></body></html>'
    const BUILT =
      '<html><head><script type="module" crossorigin src="/assets/index-abc.js"></script></head><body><app-root></app-root></body></html>'

    const roots: string[] = []
    afterAll(() => roots.forEach((root) => rmSync(root, { recursive: true, force: true })))

    /** A root with the source index.html and, when given, a built `dist/client/index.html`. */
    function tempRoot(built?: string): string {
      const root = mkdtempSync(join(tmpdir(), 'oxc-angular-ssr-manifest-'))
      roots.push(root)
      writeFileSync(join(root, 'index.html'), SOURCE)
      if (built) {
        mkdirSync(join(root, 'dist/client'), { recursive: true })
        writeFileSync(join(root, 'dist/client/index.html'), built)
      }
      return root
    }

    /** Runs the client's generateBundle (when given a bundle), then the server build. */
    async function serverBuild(
      root: string,
      { bundle, builder }: { bundle?: Record<string, unknown>; builder?: boolean } = {},
    ) {
      const plugin = ssrManifestPlugin({}) as any
      plugin.configResolved({ root, base: '/' })
      if (bundle) {
        plugin.generateBundle.handler.call(
          { environment: { config: { consumer: 'client' } } },
          {},
          bundle,
        )
      }
      const warnings: string[] = []
      const context = {
        environment: {
          config: {
            consumer: 'server',
            root,
            environments: { client: { build: { outDir: 'dist/client' } } },
          },
          getTopLevelConfig: () => ({ builder: builder ? {} : undefined }),
        },
        warn: (message: string) => warnings.push(message),
      }
      await plugin.buildStart.call(context)
      const code: string = plugin.transform.call(
        context,
        'const engine = new AngularNodeAppEngine();',
        join(root, 'src/server.ts'),
      ).code
      return { code, warnings }
    }

    it('shares one plugin instance across environments', () => {
      expect(ssrManifestPlugin({}).sharedDuringBuild).toBe(true)
    })

    it('embeds the index.html the client built in the same run', async () => {
      for (const source of [BUILT, new TextEncoder().encode(BUILT)]) {
        const { code, warnings } = await serverBuild(tempRoot(), {
          bundle: { 'index.html': { type: 'asset', fileName: 'index.html', source } },
          builder: true,
        })
        expect(code).toContain('/assets/index-abc.js')
        expect(code).not.toContain('/src/main.ts')
        expect(warnings).toEqual([])
      }
    })

    it('ignores an index.html in a server bundle', async () => {
      const plugin = ssrManifestPlugin({}) as any
      plugin.configResolved({ root: '/app', base: '/' })
      plugin.generateBundle.handler.call(
        { environment: { config: { consumer: 'server' } } },
        {},
        { 'index.html': { type: 'asset', fileName: 'index.html', source: BUILT } },
      )
      const { code } = await serverBuild(tempRoot())
      expect(code).toContain('/src/main.ts')
    })

    it("reads the client outDir's index.html from an earlier run", async () => {
      const { code, warnings } = await serverBuild(tempRoot(BUILT))
      expect(code).toContain('/assets/index-abc.js')
      expect(warnings).toEqual([])
    })

    it('warns that the page may be stale when the server builds first with --app', async () => {
      const root = tempRoot(BUILT)
      const { code, warnings } = await serverBuild(root, { builder: true })
      expect(code).toContain('/assets/index-abc.js')
      expect(warnings).toHaveLength(1)
      expect(warnings[0]).toContain('may be stale')
      expect(warnings[0]).toContain(join(root, 'dist/client/index.html'))
    })

    it('falls back to the source index.html with a warning', async () => {
      const root = tempRoot()
      const { code, warnings } = await serverBuild(root)
      expect(code).toContain('/src/main.ts')
      expect(warnings).toHaveLength(1)
      expect(warnings[0]).toContain(join(root, 'dist/client/index.html'))
      expect(warnings[0]).toContain('vite build --app')

      // With --app, the client was not built first.
      const app = await serverBuild(tempRoot(), { builder: true })
      expect(app.code).toContain('/src/main.ts')
      expect(app.warnings).toHaveLength(1)
      expect(app.warnings[0]).toContain('List `client` first')
    })

    it('falls back to a minimal page without any index.html', async () => {
      const root = tempRoot()
      rmSync(join(root, 'index.html'))
      const { code, warnings } = await serverBuild(root)
      expect(code).toContain('<app-root></app-root>')
      expect(warnings).toEqual([])
    })
  })
})

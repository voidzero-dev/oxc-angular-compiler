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

    const ENGINE = 'const engine = new AngularNodeAppEngine();'
    const page = (source: string) => ({
      'index.html': { type: 'asset', fileName: 'index.html', source },
    })

    /** One plugin shared by a `client` and an `ssr` environment, listed in `order`. */
    function app(
      root: string,
      {
        builder,
        watch,
        order = ['client', 'ssr'],
      }: { builder?: boolean; watch?: boolean; order?: string[] } = {},
    ) {
      const plugin = ssrManifestPlugin({}) as any
      plugin.configResolved({ root, base: '/' })
      const environments = Object.fromEntries(
        order.map((name) => [name, name === 'client' ? { build: { outDir: 'dist/client' } } : {}]),
      )
      const warnings: string[] = []
      const context = (name: string) => ({
        environment: {
          name,
          config: {
            consumer: name === 'client' ? 'client' : 'server',
            root,
            build: { watch: watch ? {} : null },
            environments,
          },
          getTopLevelConfig: () => ({ builder: builder ? {} : undefined }),
        },
        meta: { watchMode: !!watch },
        warn: (message: string) => warnings.push(message),
      })
      return {
        warnings,
        clientStart: () => plugin.buildStart.call(context('client')),
        clientBundle: (bundle: Record<string, unknown>) =>
          plugin.generateBundle.handler.call(context('client'), {}, bundle),
        /** The server's buildStart, then a transform of each module. */
        async server(...modules: string[]): Promise<(string | undefined)[]> {
          await plugin.buildStart.call(context('ssr'))
          return modules.map(
            (code, i) =>
              plugin.transform.call(context('ssr'), code, join(root, `src/server${i}.ts`))?.code,
          )
        },
      }
    }

    /** Runs the client's generateBundle (when given a bundle), then the server build. */
    async function serverBuild(
      root: string,
      {
        bundle,
        builder,
        order,
      }: { bundle?: Record<string, unknown>; builder?: boolean; order?: string[] } = {},
    ) {
      const build = app(root, { builder, order })
      if (bundle) build.clientBundle(bundle)
      const [code] = await build.server(ENGINE)
      return { code: code!, warnings: build.warnings }
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
      const { code, warnings } = await serverBuild(root, {
        builder: true,
        order: ['ssr', 'client'],
      })
      expect(code).toContain('/assets/index-abc.js')
      expect(warnings).toHaveLength(1)
      expect(warnings[0]).toContain('may be stale')
      expect(warnings[0]).toContain(join(root, 'dist/client/index.html'))
      expect(warnings[0]).toContain('List `client` first')
    })

    it('falls back to the source index.html with a warning', async () => {
      const root = tempRoot()
      const { code, warnings } = await serverBuild(root)
      expect(code).toContain('/src/main.ts')
      expect(warnings).toHaveLength(1)
      expect(warnings[0]).toContain(join(root, 'dist/client/index.html'))
      expect(warnings[0]).toContain('vite build --app')

      // With --app, the client was not built first.
      const ssrFirst = await serverBuild(tempRoot(), { builder: true, order: ['ssr', 'client'] })
      expect(ssrFirst.code).toContain('/src/main.ts')
      expect(ssrFirst.warnings).toHaveLength(1)
      expect(ssrFirst.warnings[0]).toContain('List `client` first')

      // The client is listed first but built no index.html.
      const clientFirst = await serverBuild(tempRoot(), { builder: true })
      expect(clientFirst.code).toContain('/src/main.ts')
      expect(clientFirst.warnings).toHaveLength(1)
      expect(clientFirst.warnings[0]).not.toContain('List `client` first')
    })

    it('warns only when a manifest embeds the page, once per build', async () => {
      const build = app(tempRoot())
      expect(await build.server('export function render() {}')).toEqual([undefined])
      expect(build.warnings).toEqual([])

      const codes = await build.server('export function render() {}', ENGINE, ENGINE)
      expect(codes[0]).toBeUndefined()
      expect(codes[1]).toContain('/src/main.ts')
      expect(codes[2]).toContain('/src/main.ts')
      expect(build.warnings).toHaveLength(1)
      expect(build.warnings[0]).toContain('built client index.html not found')

      await build.server(ENGINE)
      expect(build.warnings).toHaveLength(2)
    })

    it('does not silently reuse the previous round in watch mode', async () => {
      // `vite build --app --watch` builds the environments at the same time.
      const root = tempRoot()
      const build = app(root, { builder: true, watch: true })
      const NEXT = BUILT.replace('index-abc', 'index-def')

      // Round 1: the server starts before the client has built.
      await build.clientStart()
      const [first] = await build.server(ENGINE)
      build.clientBundle(page(BUILT))
      expect(first).toContain('/src/main.ts')
      expect(build.warnings).toHaveLength(1)
      expect(build.warnings[0]).toContain('In watch mode')
      expect(build.warnings[0]).not.toContain('List `client` first')

      // Round 2: the client starts first and drops its old page, so the server
      // reads the client outDir of round 1.
      mkdirSync(join(root, 'dist/client'), { recursive: true })
      writeFileSync(join(root, 'dist/client/index.html'), BUILT)
      await build.clientStart()
      const [second] = await build.server(ENGINE)
      build.clientBundle(page(NEXT))
      expect(second).toContain('/assets/index-abc.js')
      expect(build.warnings).toHaveLength(2)
      expect(build.warnings[1]).toContain('may be stale')
      expect(build.warnings[1]).toContain('In watch mode')

      // Round 3: the server starts first, with the page of round 2.
      const [third] = await build.server(ENGINE)
      await build.clientStart()
      expect(third).toContain('/assets/index-def.js')
      expect(build.warnings).toHaveLength(3)
      expect(build.warnings[2]).toContain('may be stale')
      expect(build.warnings[2]).toContain('In watch mode')
    })

    it('embeds the page of this round without a warning outside watch mode', async () => {
      const build = app(tempRoot(), { builder: true })
      await build.clientStart()
      build.clientBundle(page(BUILT))
      const [code] = await build.server(ENGINE)
      expect(code).toContain('/assets/index-abc.js')
      expect(build.warnings).toEqual([])
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

import { mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { basename, dirname, join } from 'node:path'
import { runInNewContext } from 'node:vm'

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
      await (plugin.buildStart as any).handler.call({})
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
    // The dirs of the watched files of watch mode, in the temp dir.
    const stamps: string[] = []
    afterAll(() =>
      [...roots, ...stamps].forEach((path) => rmSync(path, { recursive: true, force: true })),
    )

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

    /**
     * One plugin shared by a `client` and an `ssr` environment, listed in
     * `order`. `watch` is true for all environments or names the ones that watch.
     */
    function app(
      root: string,
      {
        builder,
        watch,
        order = ['client', 'ssr'],
      }: { builder?: boolean; watch?: boolean | string[]; order?: string[] } = {},
    ) {
      const plugin = ssrManifestPlugin({}) as any
      plugin.configResolved({ root, base: '/' })
      const environments = Object.fromEntries(
        order.map((name) => [name, name === 'client' ? { build: { outDir: 'dist/client' } } : {}]),
      )
      const warnings: string[] = []
      const watched: string[] = []
      const watches = (name: string) =>
        watch === true || (Array.isArray(watch) && watch.includes(name))
      const context = (name: string, consumer = name === 'ssr' ? 'server' : 'client') => ({
        environment: {
          name,
          config: {
            consumer,
            root,
            build: { watch: watches(name) ? {} : null },
            environments,
          },
          getTopLevelConfig: () => ({ builder: builder ? {} : undefined }),
        },
        meta: { watchMode: watches(name) },
        warn: (message: string) => warnings.push(message),
        addWatchFile: (file: string) => {
          watched.push(file)
          // The plugin removes its dir only on a normal process exit.
          stamps.push(dirname(file))
        },
      })
      return {
        plugin,
        warnings,
        watched,
        clientStart: () => plugin.buildStart.handler.call(context('client')),
        /** The generateBundle of the `client` environment, or of another one (`name`). */
        clientBundle: (bundle: Record<string, unknown>, name = 'client') =>
          plugin.generateBundle.handler.call(context(name), {}, bundle),
        clientEnd: (error?: Error) => plugin.buildEnd.call(context('client'), error),
        serverEnd: (error?: Error) => plugin.buildEnd.call(context('ssr'), error),
        serverBundle: () => plugin.generateBundle.handler.call(context('ssr'), {}, {}),
        clientRenderError: () => plugin.renderError.call(context('client'), new Error('client')),
        /** The server's buildStart, then a transform of each module. */
        async server(...modules: string[]): Promise<(string | undefined)[]> {
          await plugin.buildStart.handler.call(context('ssr'))
          return modules.map(
            (code, i) =>
              plugin.transform.call(context('ssr'), code, join(root, `src/server${i}.ts`))?.code,
          )
        },
        /** The server's renderChunk of a chunk with `code`. */
        async render(code: string): Promise<string | undefined> {
          return (await plugin.renderChunk.call(context('ssr'), code))?.code
        },
      }
    }

    /** Whether `promise` settles within a short time. */
    async function settles(promise: Promise<unknown>): Promise<boolean> {
      let done = false
      promise.then(
        () => (done = true),
        () => (done = true),
      )
      await new Promise((r) => setTimeout(r, 50))
      return done
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
      if (bundle) await build.clientBundle(bundle)
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

    describe('vite build --app --watch (#705)', () => {
      // The Angular CLI builds index.server.html from the browser index.html of
      // the same build (@angular/build execute-post-bundle.js), so each server
      // round must embed the page of the same client round.
      const NEXT = BUILT.replace('index-abc', 'index-def')
      const filled = (html: string) => 'Promise.resolve(`' + html + '`)'
      const watchApp = (root = tempRoot()) => app(root, { builder: true, watch: true })

      /** Runs the plugin's buildApp, then `builder.build` of the client, which returns a watcher. */
      async function startClientWatcher(build: ReturnType<typeof app>) {
        const listeners: ((event: { code: string }) => void)[] = []
        const watcher = {
          on: (_: string, listener: (event: { code: string }) => void) => listeners.push(listener),
        }
        const builder = { environments: { client: { isBuilt: false } }, build: async () => watcher }
        await build.plugin.buildApp.handler.call({}, builder)
        // The plugin wraps `build`, which takes the environment.
        await (builder.build as (environment: { name: string }) => Promise<unknown>)({
          name: 'client',
        })
        return (code: string) => listeners.forEach((listener) => listener({ code }))
      }

      it('fills in the page of the same client round', async () => {
        const build = watchApp()

        // Round 1: the server renders while the client round runs.
        await build.clientStart()
        const [code] = await build.server(ENGINE)
        expect(code).toContain('size: __oxc_angular_index_server_html_size__,')
        expect(code).toContain('text: () => Promise.resolve(__oxc_angular_index_server_html__),')
        expect(code).not.toContain('/src/main.ts')
        const first = build.render(code!)
        build.clientEnd()
        expect(await settles(first)).toBe(false)
        await build.clientBundle(page(BUILT))
        const out = await first
        expect(out).toContain('size: 130,')
        expect(out).toContain(filled(BUILT))
        expect(out).not.toContain('__oxc_angular')
        // The server waited for this page, so the client does not rebuild it.
        expect(build.watched).toHaveLength(1)
        expect(readFileSync(build.watched[0], 'utf-8')).toBe('')

        // Round 2: a server-only change takes the latest page at once.
        const [serverOnly] = await build.server(ENGINE)
        expect(await build.render(serverOnly!)).toContain(filled(BUILT))

        // Round 3: a shared change waits for the new page.
        await build.clientStart()
        const [shared] = await build.server(ENGINE)
        const third = build.render(shared!)
        expect(await settles(third)).toBe(false)
        await build.clientBundle(page(NEXT))
        expect(await third).toContain(filled(NEXT))
        expect(build.warnings).toEqual([])
      })

      it('waits for the first client round once the client watcher exists', async () => {
        for (const how of ['build', 'isBuilt']) {
          const build = watchApp()
          if (how === 'build') {
            await startClientWatcher(build)
          } else {
            await build.plugin.buildApp.handler.call(
              {},
              { environments: { client: { isBuilt: true } }, build: async () => ({}) },
            )
          }
          const [code] = await build.server(ENGINE)
          const rendered = build.render(code!)
          expect(await settles(rendered)).toBe(false)
          await build.clientStart()
          await build.clientBundle(page(NEXT))
          expect(await rendered).toContain(filled(NEXT))
          expect(build.warnings).toEqual([])
        }
      })

      it('does not wait without a client watcher', async () => {
        // A custom buildApp that builds only the server.
        const root = tempRoot()
        const build = watchApp(root)
        const [code] = await build.server(ENGINE)
        expect(await build.render(code!)).toContain('/src/main.ts')
        expect(build.warnings).toEqual([
          `built client index.html not found at ${join(root, 'dist/client/index.html')}; ` +
            `embedding the source ${join(root, 'index.html')}, so SSR pages will load the source entry.`,
        ])

        const built = tempRoot(BUILT)
        const earlier = watchApp(built)
        const [again] = await earlier.server(ENGINE)
        expect(await earlier.render(again!)).toContain(filled(BUILT))
        expect(earlier.warnings).toEqual([
          `${join(built, 'dist/client/index.html')} comes from an earlier client build and may be stale.`,
        ])
      })

      it('does not block the server when a client round fails', async () => {
        // An error in another plugin's generateBundle shows only as the watcher's ERROR event.
        for (const how of ['buildEnd', 'renderError', 'ERROR event']) {
          const build = watchApp()
          const emit = await startClientWatcher(build)
          await build.clientStart()
          await build.clientBundle(page(BUILT))

          await build.clientStart()
          const [code] = await build.server(ENGINE)
          const rendered = build.render(code!)
          expect(await settles(rendered)).toBe(false)
          if (how === 'buildEnd') build.clientEnd(new Error('client'))
          else if (how === 'renderError') build.clientRenderError()
          else emit('ERROR')
          expect(await rendered).toContain(filled(BUILT))
          expect(build.warnings).toEqual([
            'the client build failed, so the SSR page comes from its previous build and may be stale.',
          ])
        }
      })

      it('rebuilds the server through a watched file when the client page changes', async () => {
        // Rolldown never watches a path that is missing when the build ends, so
        // the client's index.html in a fresh outDir would never rebuild it.
        const build = watchApp()
        await startClientWatcher(build)

        // Round 1 of a fresh outDir: the client fails, so the server embeds the source page.
        await build.clientStart()
        const [code] = await build.server(ENGINE)
        expect(build.watched).toHaveLength(1)
        const [stamp] = build.watched
        expect(readFileSync(stamp, 'utf-8')).toBe('')
        // A new private dir, so no other user can plant a link there or make it read-only.
        expect(basename(dirname(stamp))).toMatch(/^oxc-angular-ssr-.{6}$/)
        // Windows has no mode bits: a new dir reads as 0o777 there.
        if (process.platform !== 'win32') expect(statSync(dirname(stamp)).mode & 0o777).toBe(0o700)
        const rendered = build.render(code!)
        build.clientEnd(new Error('client'))
        expect(await rendered).toContain('/src/main.ts')

        // A client-only fix: the server embedded another page, so the client writes the file.
        await build.clientStart()
        await build.clientBundle(page(BUILT))
        expect(readFileSync(stamp, 'utf-8')).not.toBe('')

        // The rebuilt server takes the page, and the same page again writes nothing.
        writeFileSync(stamp, 'seen')
        const [again] = await build.server(ENGINE)
        expect(build.watched).toEqual([stamp, stamp])
        expect(await build.render(again!)).toContain(filled(BUILT))
        await build.clientStart()
        await build.clientBundle(page(BUILT))
        expect(readFileSync(stamp, 'utf-8')).toBe('seen')

        // Outside watch mode nothing is watched.
        const once = app(tempRoot(), { builder: true })
        await once.server(ENGINE)
        expect(once.watched).toEqual([])
      })

      it('does not rebuild a server whose build takes the new page by itself', async () => {
        const build = watchApp()
        await build.clientStart()
        const [code] = await build.server(ENGINE)
        const [stamp] = build.watched
        const rendered = build.render(code!)
        await build.clientBundle(page(BUILT))
        expect(await rendered).toContain(filled(BUILT))
        await build.serverBundle()

        // A shared change, with a server slower than the client: the client
        // round ends while the server is still before its renderChunk.
        await build.clientStart()
        const [slow] = await build.server(ENGINE)
        await build.clientBundle(page(NEXT))
        expect(readFileSync(stamp, 'utf-8')).toBe('')
        expect(await build.render(slow!)).toContain(filled(NEXT))
        await build.serverBundle()

        // A server build that fails takes no page, so the next client change rebuilds it.
        await build.server(ENGINE)
        build.serverEnd(new Error('server'))
        await build.clientStart()
        await build.clientBundle(page(BUILT))
        expect(readFileSync(stamp, 'utf-8')).not.toBe('')
        expect(build.warnings).toEqual([])
      })

      it('warns once when it cannot make the watched file', async () => {
        const root = tempRoot()
        const missing = join(root, 'missing')
        // os.tmpdir() reads TMPDIR on POSIX, TEMP or TMP on Windows.
        const names = ['TMPDIR', 'TEMP', 'TMP'] as const
        const saved = names.map((name) => process.env[name])
        // Only the plugin's temp dir is missing: the root is made before.
        for (const name of names) process.env[name] = missing
        try {
          const build = watchApp(root)
          await build.server(ENGINE)
          await build.server(ENGINE)
          expect(build.watched).toEqual([])
          expect(build.warnings).toHaveLength(1)
          expect(build.warnings[0]).toContain(
            `could not make a file in ${missing}, so client-only changes do not rebuild the server: `,
          )
        } finally {
          for (const [i, name] of names.entries()) {
            if (saved[i] === undefined) delete process.env[name]
            else process.env[name] = saved[i]
          }
        }
      })

      it('takes the page only from the environment named client', async () => {
        const build = watchApp()
        await build.clientStart()
        const [code] = await build.server(ENGINE)
        const rendered = build.render(code!)
        // Another client environment (a worker or a second browser build).
        await build.clientBundle(page(NEXT), 'legacy')
        expect(await settles(rendered)).toBe(false)
        await build.clientBundle(page(BUILT))
        expect(await rendered).toContain(filled(BUILT))
      })

      it('takes the page of a client that does not watch', async () => {
        // Only `environments.ssr.build.watch` is set: the client builds once, first.
        const build = app(tempRoot(), { builder: true, watch: ['ssr'] })
        await build.clientStart()
        await build.clientBundle(page(BUILT))
        const [code] = await build.server(ENGINE)
        expect(await build.render(code!)).toContain(filled(BUILT))
        expect(build.warnings).toEqual([])
      })

      it('fills in one line with the same text as outside watch mode', async () => {
        const PAGE = '<html>\r\n<body>`${x}\\</body>\r</html>\n'
        const build = watchApp()
        await build.clientStart()
        const [code] = await build.server(ENGINE)
        const rendered = build.render(code!)
        await build.clientBundle(page(PAGE))
        const out = (await rendered)!

        const literal = (c: string) =>
          c.slice(
            c.indexOf('Promise.resolve(') + 'Promise.resolve('.length,
            c.indexOf('),\n    },\n  },'),
          )
        expect(literal(out)).toBe('`<html>\\n<body>\\`\\${x}\\\\</body>\\n</html>\\n`')
        const outside = generateAppManifestCode({
          ssrEntryImport: './main.server',
          baseHref: '/',
          indexHtmlContent: PAGE,
        })
        expect(runInNewContext(literal(out))).toBe(runInNewContext(literal(outside)))
        expect(out).toContain('size: 36,')
        expect(outside).toContain('size: 36,')
      })
    })

    it('embeds the page of this round without a warning outside watch mode', async () => {
      const build = app(tempRoot(), { builder: true })
      await build.clientStart()
      build.clientBundle(page(BUILT))
      const [code] = await build.server(ENGINE)
      expect(code).toContain('/assets/index-abc.js')
      expect(code).not.toContain('__oxc_angular')
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

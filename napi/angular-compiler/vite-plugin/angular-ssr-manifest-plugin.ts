/**
 * Angular SSR Manifest Plugin
 *
 * Generates the Angular SSR manifests required by AngularNodeAppEngine.
 * Without these manifests, Angular throws:
 *   "Angular app engine manifest is not set."
 *
 * This plugin:
 * 1. Detects server builds (environments whose `consumer` is `server`, which
 *    includes the `ssr` environment `vite build --ssr` builds)
 * 2. Auto-injects manifest setup into files that use AngularNodeAppEngine/AngularAppEngine
 * 3. Provides the client's built index.html as a server asset for SSR rendering
 *
 * @see https://github.com/voidzero-dev/oxc-angular-compiler/issues/60
 */

import { mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { readFile, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve } from 'node:path'

import type { Plugin, ResolvedConfig, Rolldown, ViteBuilder } from 'vite'
import { normalizePath } from 'vite'

/**
 * Unsafe characters that need escaping in template literals.
 */
const UNSAFE_CHAR_MAP: Record<string, string> = {
  '`': '\\`',
  $: '\\$',
  '\\': '\\\\',
}

function escapeUnsafeChars(str: string): string {
  return str.replace(/[$`\\]/g, (c) => UNSAFE_CHAR_MAP[c])
}

/**
 * Generate the code that calls ɵsetAngularAppManifest.
 *
 * This sets up the app-level manifest with bootstrap function and server assets.
 */
export function generateAppManifestCode(options: {
  ssrEntryImport: string
  baseHref: string
  indexHtmlContent: string
}): string {
  const { ssrEntryImport, baseHref, indexHtmlContent } = options
  const escapedHtml = escapeUnsafeChars(indexHtmlContent)
  const htmlSize = Buffer.byteLength(indexHtmlContent, 'utf-8')

  return `
import { ɵsetAngularAppManifest as __oxc_setAppManifest } from '@angular/ssr';

__oxc_setAppManifest({
  bootstrap: () => import('${ssrEntryImport}').then(m => m.default),
  inlineCriticalCss: true,
  baseHref: '${baseHref}',
  locale: undefined,
  routes: undefined,
  entryPointToBrowserMapping: undefined,
  assets: {
    'index.server.html': {
      size: ${htmlSize},
      hash: '',
      text: () => Promise.resolve(\`${escapedHtml}\`),
    },
  },
});
`
}

/**
 * Generate the code that calls ɵsetAngularAppEngineManifest.
 *
 * This sets up the engine-level manifest with entry points and locale support.
 */
export function generateAppEngineManifestCode(options: { basePath: string }): string {
  let { basePath } = options

  // Remove trailing slash but retain leading slash (matching Angular behavior)
  if (basePath.length > 1 && basePath.at(-1) === '/') {
    basePath = basePath.slice(0, -1)
  }

  return `
import {
  ɵsetAngularAppEngineManifest as __oxc_setEngineManifest,
  ɵgetOrCreateAngularServerApp as __oxc_getOrCreateAngularServerApp,
  ɵdestroyAngularServerApp as __oxc_destroyAngularServerApp,
  ɵextractRoutesAndCreateRouteTree as __oxc_extractRoutesAndCreateRouteTree,
} from '@angular/ssr';

__oxc_setEngineManifest({
  basePath: '${basePath}',
  allowedHosts: [],
  supportedLocales: { '': '' },
  entryPoints: {
    '': () => Promise.resolve({
      ɵgetOrCreateAngularServerApp: __oxc_getOrCreateAngularServerApp,
      ɵdestroyAngularServerApp: __oxc_destroyAngularServerApp,
      ɵextractRoutesAndCreateRouteTree: __oxc_extractRoutesAndCreateRouteTree,
    }),
  },
});
`
}

export interface SsrManifestPluginOptions {
  /** Path to main.server.ts (the Angular SSR bootstrap file). Auto-detected if not specified. */
  ssrEntry?: string
}

// In `vite build --app --watch`, the server's manifest holds these free
// identifiers until its renderChunk fills in the page of the client round.
const PAGE_TOKEN = '__oxc_angular_index_server_html__'
const PAGE_SIZE_TOKEN = '__oxc_angular_index_server_html_size__'

const MINIMAL_PAGE = '<!DOCTYPE html><html><head></head><body><app-root></app-root></body></html>'

type Environment = NonNullable<Rolldown.PluginContext['environment']>
type EnvironmentConfig = Environment['config']

/** One build of the `client` environment, while the server may wait for its page. */
interface ClientRound {
  promise: Promise<void>
  resolve: () => void
  /** The round ended with an error, so it built no page. */
  failed?: boolean
}

/** What a watched server environment embedded, to know when it must rebuild. */
interface ServerState {
  /** The page its last round embedded. */
  page?: string
  /** Its renderChunk calls that wait for a client round. */
  waiting: number
  /**
   * It is between buildStart and the point where renderChunk takes the page.
   * Such a build takes the newest client page by itself, so it needs no rebuild.
   */
  building: boolean
  /** It gave the warning of this round. */
  warned: boolean
}

// The dirs of the files below, removed when the process exits normally. Vite
// sets no signal handler for `vite build --watch`, so a watch stopped with
// Ctrl+C leaves its empty dir to the system's temp cleanup. A signal handler
// here would change how the process exits, so there is none.
const stampDirs: string[] = []

/**
 * Makes the file a watched server watches. It lives in a new private dir
 * (mode 0700), so no other user can plant a link there or make it read-only.
 * The dir is in the temp dir, by its real path: rolldown does not watch files
 * in node_modules (Vite's cacheDir), and on macOS it missed changes to a file
 * under the /var symlink of the temp dir.
 */
function createStampFile(): string {
  const dir = mkdtempSync(join(realpathSync(tmpdir()), 'oxc-angular-ssr-'))
  if (stampDirs.length === 0) {
    process.once('exit', () =>
      stampDirs.forEach((path) => rmSync(path, { recursive: true, force: true })),
    )
  }
  stampDirs.push(dir)
  const path = join(dir, 'client.stamp')
  writeFileSync(path, '', { flag: 'wx' })
  return path
}

function newClientRound(): ClientRound {
  let resolve!: () => void
  const promise = new Promise<void>((r) => (resolve = r))
  return { promise, resolve }
}

function sourceText(source: string | Uint8Array): string {
  return typeof source === 'string' ? source : Buffer.from(source).toString()
}

function isBuilder(environment: Environment): boolean {
  return !!environment.getTopLevelConfig?.().builder
}

function isWatch(environment: Environment, meta: Rolldown.PluginContextMeta | undefined): boolean {
  return !!(meta?.watchMode || environment.config.build?.watch)
}

/**
 * A server environment of `vite build --app --watch`. The client and server
 * environments build at the same time, so its page is filled in at render time.
 */
function isAppWatchServer(
  environment: Environment,
  meta: Rolldown.PluginContextMeta | undefined,
): boolean {
  return (
    environment.config.consumer === 'server' &&
    isWatch(environment, meta) &&
    isBuilder(environment) &&
    !!environment.config.environments?.client
  )
}

/** The `client` environment of `vite build --app`, the one whose page the server embeds. */
function isAppClient(environment: Environment): boolean {
  return (
    environment.name === 'client' &&
    environment.config.consumer === 'client' &&
    isBuilder(environment)
  )
}

/**
 * The page to embed when the client built none in this run: the client
 * outDir's index.html from an earlier run, the source index.html, or a
 * minimal page. `warning` says why the page may be wrong.
 */
async function fallbackPage(
  config: EnvironmentConfig,
  builder: boolean,
  advice: string,
): Promise<{ page: string; warning?: string }> {
  // A client built by an earlier run. buildStart runs before Vite empties outDir.
  const clientOutDir = config.environments?.client?.build?.outDir
  const builtPath = clientOutDir && resolve(config.root, clientOutDir, 'index.html')
  if (builtPath) {
    try {
      const page = await readFile(builtPath, 'utf-8')
      return {
        page,
        warning: builder
          ? `${builtPath} comes from an earlier client build and may be stale.` + advice
          : undefined,
      }
    } catch {
      // Not built yet.
    }
  }

  const sourcePath = resolve(config.root, 'index.html')
  try {
    const page = await readFile(sourcePath, 'utf-8')
    return {
      page,
      warning:
        `built client index.html not found${builtPath ? ` at ${builtPath}` : ''}; ` +
        `embedding the source ${sourcePath}, so SSR pages will load the source entry.` +
        (builder
          ? advice
          : ' Build the client first (`vite build --app`) or set ' +
            '`environments.client.build.outDir`.'),
    }
  } catch {
    // index.html not found, provide a minimal fallback
    return { page: MINIMAL_PAGE }
  }
}

/**
 * Vite plugin that generates Angular SSR manifests for AngularNodeAppEngine.
 */
export function ssrManifestPlugin(options: SsrManifestPluginOptions): Plugin {
  let resolvedConfig: ResolvedConfig
  let ssrEntryPath: string
  let indexHtmlContent: string | undefined
  // The client environment's built index.html, when it was built first in this run.
  let clientIndexHtml: Rolldown.OutputAsset | undefined
  // The fallback warning of each server environment, emitted once it injects the manifest.
  const fallbackWarnings = new Map<string | undefined, string>()

  // `vite build --app --watch` (#705). Like the Angular CLI, each server round
  // must embed the page of the same client round, but the two environments
  // build at the same time. So the server waits for that round in renderChunk.

  // The page the `client` environment built last in this run.
  let clientPage: string | undefined
  // The client round in progress.
  let clientRound: ClientRound | undefined
  // The first client round ended (with or without a page).
  let firstRoundDone = false
  const firstRound = newClientRound()
  let appBuilder: ViteBuilder | undefined
  // `builder.build` was called for the client, so its first round will come.
  let clientBuildStarted = false
  // A file each watched server environment watches. Rolldown has no call that
  // starts a rebuild, so the client writes this file when a server needs its page.
  let stampPath: string | undefined
  // Making the file failed once, so do not try (and warn) again.
  let stampFailed = false
  const servers = new Map<string, ServerState>()

  const serverState = (name: string): ServerState => {
    let state = servers.get(name)
    if (!state) servers.set(name, (state = { waiting: 0, building: false, warned: false }))
    return state
  }

  const endClientRound = (failed: boolean) => {
    if (clientRound) {
      clientRound.failed = failed
      clientRound.resolve()
      clientRound = undefined
    }
    if (!firstRoundDone) {
      firstRoundDone = true
      firstRound.resolve()
    }
  }

  return {
    name: '@oxc-angular/vite-ssr-manifest',
    apply: 'build',
    // One instance for all environments (`vite build --app`), so the server
    // build sees the client's index.html.
    sharedDuringBuild: true,

    configResolved(config) {
      resolvedConfig = config

      const workspaceRoot = config.root

      // Determine the SSR bootstrap entry (main.server.ts)
      ssrEntryPath = options.ssrEntry
        ? resolve(workspaceRoot, options.ssrEntry)
        : resolve(workspaceRoot, 'src/main.server.ts')
    },

    buildApp: {
      order: 'pre',
      async handler(builder) {
        appBuilder = builder
        // In watch mode `build` returns the client's watcher. A round that fails
        // in a hook this plugin does not see (another plugin's generateBundle)
        // only shows up as an ERROR event, and the server must not wait forever.
        const build = builder.build.bind(builder)
        builder.build = async (environment) => {
          const client = environment.name === 'client'
          if (client) clientBuildStarted = true
          const output = await build(environment)
          if (client && output && 'on' in output && typeof output.on === 'function') {
            output.on('event', (event) => {
              if (event.code === 'ERROR') endClientRound(true)
            })
          }
          return output
        }
      },
    },

    generateBundle: {
      // 'post': vite:build-html adds index.html to the bundle in its own generateBundle.
      order: 'post',
      async handler(_, bundle) {
        const environment = this.environment
        if (environment?.config.consumer === 'server') {
          // A build with no manifest to fill in has no renderChunk that takes the page.
          const server = servers.get(environment.name)
          if (server) server.building = false
          return
        }
        if (environment?.config.consumer !== 'client') return
        const asset = bundle['index.html']
        if (asset?.type === 'asset') clientIndexHtml = asset
        if (!isAppClient(environment)) return

        if (asset?.type === 'asset') clientPage = sourceText(asset.source)
        // A watched server that embedded another page, and is not building
        // now (a build in progress takes this page in renderChunk), must rebuild.
        const page = clientPage
        const rebuild =
          page !== undefined &&
          [...servers.values()].some(
            (server) =>
              server.page !== undefined &&
              !server.building &&
              server.waiting === 0 &&
              server.page !== page,
          )
        endClientRound(false)
        if (rebuild && stampPath) {
          try {
            await writeFile(stampPath, `${Date.now()}\n`)
          } catch (error) {
            this.warn(
              `could not write ${stampPath}, so the server does not rebuild: ${String(error)}`,
            )
          }
        }
      },
    },

    buildEnd(error) {
      if (!error || !this.environment) return
      if (isAppClient(this.environment)) endClientRound(true)
      // A failed server build takes no page, so a client change must rebuild it.
      const server = servers.get(this.environment.name)
      if (server) server.building = false
    },

    renderError() {
      if (!this.environment) return
      if (isAppClient(this.environment)) endClientRound(true)
      const server = servers.get(this.environment.name)
      if (server) server.building = false
    },

    closeBundle() {
      // Outside watch mode, a failed generateBundle still closes the bundle. In
      // watch mode the watcher's ERROR event ends the round.
      const environment = this.environment
      if (environment && isAppClient(environment) && !isWatch(environment, this.meta)) {
        endClientRound(true)
      }
    },

    buildStart: {
      // 'pre': open the client round before slower buildStart hooks run.
      order: 'pre',
      async handler() {
        // `environment` is only set by Vite, not by plain rolldown / tsdown.
        if (!this.environment) return
        const config = this.environment.config
        if (config.consumer === 'client') {
          if (isAppClient(this.environment) && !clientRound) clientRound = newClientRound()
          // A watch round must not reuse the page of the previous round.
          if (isWatch(this.environment, this.meta)) clientIndexHtml = undefined
          return
        }
        if (config.consumer !== 'server') return
        const name = this.environment.name
        fallbackWarnings.delete(name)

        if (isAppWatchServer(this.environment, this.meta)) {
          // The page comes in renderChunk. Watch a file that exists before this
          // build ends (rolldown never watches a path that was missing then),
          // so a client-only change can rebuild the server.
          const server = serverState(name)
          server.warned = false
          server.building = true
          if (!stampPath && !stampFailed) {
            try {
              stampPath = createStampFile()
            } catch (error) {
              stampFailed = true
              this.warn(
                `could not make a file in ${tmpdir()}, so client-only changes do not rebuild the server: ${String(error)}`,
              )
            }
          }
          if (stampPath) this.addWatchFile(stampPath)
          return
        }

        // `vite build --app` builds the environments in `environments` order.
        const builder = isBuilder(this.environment)
        const names = Object.keys(config.environments ?? {})
        const clientFirst = names.indexOf('client') < names.indexOf(name)
        const advice = builder && !clientFirst ? ' List `client` first in `environments`.' : ''

        // Like the Angular CLI, serve the built browser page, which loads the
        // hashed bundles, not the source one (@angular/build
        // src/utils/index-file/index-html-generator.ts builds index.server.html
        // from the augmented page).
        if (clientIndexHtml) {
          indexHtmlContent = sourceText(clientIndexHtml.source)
          return
        }

        const fallback = await fallbackPage(config, builder, advice)
        indexHtmlContent = fallback.page
        if (fallback.warning) fallbackWarnings.set(name, fallback.warning)
      },
    },

    transform(code, id) {
      if (this.environment?.config.consumer !== 'server') return

      // Inject manifest setup into files that use AngularNodeAppEngine or AngularAppEngine
      // These are the SSR entry points that need the manifest before constructing the engine.
      // Code that already sets the manifest is skipped: a server environment can bundle
      // the output of another one (Nitro builds the `ssr` service, then bundles it).
      if (
        !id.includes('node_modules') &&
        !id.startsWith('\0') &&
        (code.includes('AngularNodeAppEngine') || code.includes('AngularAppEngine')) &&
        !code.includes('ɵsetAngularAppManifest')
      ) {
        const baseHref = resolvedConfig.base || '/'

        // Compute the import path for main.server.ts relative to the current file
        const fileDir = dirname(id)
        let ssrEntryImport = normalizePath(relative(fileDir, ssrEntryPath)).replace(/\.ts$/, '')
        if (!ssrEntryImport.startsWith('.')) {
          ssrEntryImport = './' + ssrEntryImport
        }

        // In `vite build --app --watch`, renderChunk fills in the page.
        const appManifest = isAppWatchServer(this.environment, this.meta)
          ? generateAppManifestCode({ ssrEntryImport, baseHref, indexHtmlContent: '' })
              .replace('size: 0,', `size: ${PAGE_SIZE_TOKEN},`)
              .replace('Promise.resolve(``)', `Promise.resolve(${PAGE_TOKEN})`)
          : generateAppManifestCode({
              ssrEntryImport,
              baseHref,
              indexHtmlContent: indexHtmlContent || '',
            })

        const engineManifest = generateAppEngineManifestCode({
          basePath: baseHref,
        })

        // Only warn about the page when a manifest embeds it, once per build.
        const warning = fallbackWarnings.get(this.environment.name)
        if (warning) {
          fallbackWarnings.delete(this.environment.name)
          this.warn(warning)
        }

        return {
          code: appManifest + engineManifest + code,
          map: null,
        }
      }
    },

    async renderChunk(code) {
      const environment = this.environment
      if (environment?.config.consumer !== 'server' || !code.includes(PAGE_TOKEN)) return null
      const server = serverState(environment.name)

      // Wait for the client round in progress, or for the first one when the
      // client watcher has started but its first round has not begun yet.
      let round: ClientRound | undefined
      server.waiting++
      try {
        if (clientRound) {
          round = clientRound
          await round.promise
        } else if (
          !firstRoundDone &&
          (clientBuildStarted || appBuilder?.environments.client?.isBuilt)
        ) {
          await firstRound.promise
        }
      } finally {
        server.waiting--
      }

      server.building = false
      let page = clientPage
      let warning: string | undefined
      if (page === undefined) {
        ;({ page, warning } = await fallbackPage(environment.config, true, ''))
      } else if (round?.failed) {
        warning =
          'the client build failed, so the SSR page comes from its previous build and may be stale.'
      }
      server.page = page
      if (warning && !server.warned) {
        server.warned = true
        this.warn(warning)
      }

      // One line, so the code after it keeps its line in the source map. A
      // template literal reads CRLF and CR as LF, so the text is the same as
      // the multi-line literal outside watch mode.
      const literal = '`' + escapeUnsafeChars(page).replace(/\r\n?|\n/g, '\\n') + '`'
      return {
        code: code
          .replaceAll(PAGE_SIZE_TOKEN, String(Buffer.byteLength(page, 'utf-8')))
          .replaceAll(PAGE_TOKEN, () => literal),
        map: null,
      }
    },
  }
}

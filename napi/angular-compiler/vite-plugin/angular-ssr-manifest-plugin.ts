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

import { readFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'

import type { Plugin, ResolvedConfig, Rolldown } from 'vite'
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

    generateBundle: {
      // 'post': vite:build-html adds index.html to the bundle in its own generateBundle.
      order: 'post',
      handler(_, bundle) {
        if (this.environment?.config.consumer !== 'client') return
        const asset = bundle['index.html']
        if (asset?.type === 'asset') clientIndexHtml = asset
      },
    },

    async buildStart() {
      // `environment` is only set by Vite, not by plain rolldown / tsdown.
      if (!this.environment) return
      const config = this.environment.config
      const watch = !!(this.meta?.watchMode || config.build?.watch)
      if (config.consumer === 'client') {
        // A watch round must not reuse the page of the previous round.
        if (watch) clientIndexHtml = undefined
        return
      }
      if (config.consumer !== 'server') return
      const name = this.environment.name
      fallbackWarnings.delete(name)

      // `vite build --app` builds the environments in `environments` order, one
      // after another, but in watch mode at the same time, so the client's page
      // can be the one of its previous round.
      const builder = !!this.environment.getTopLevelConfig?.().builder
      const names = Object.keys(config.environments ?? {})
      const clientFirst = names.indexOf('client') < names.indexOf(name)
      const advice =
        (builder && watch
          ? ' In watch mode the client and server environments build at the same time, ' +
            'so the SSR page may come from the previous client round or the source page.'
          : '') + (builder && !clientFirst ? ' List `client` first in `environments`.' : '')

      // Like the Angular CLI, serve the built browser page, which loads the
      // hashed bundles, not the source one (@angular/build
      // src/utils/index-file/index-html-generator.ts builds index.server.html
      // from the augmented page).
      if (clientIndexHtml) {
        const { source } = clientIndexHtml
        indexHtmlContent = typeof source === 'string' ? source : Buffer.from(source).toString()
        if (builder && watch) {
          fallbackWarnings.set(name, 'the built client index.html may be stale.' + advice)
        }
        return
      }

      // A client built by an earlier run. buildStart runs before Vite empties outDir.
      const clientOutDir = config.environments?.client?.build?.outDir
      const builtPath = clientOutDir && resolve(config.root, clientOutDir, 'index.html')
      if (builtPath) {
        try {
          indexHtmlContent = await readFile(builtPath, 'utf-8')
          if (builder) {
            fallbackWarnings.set(
              name,
              `${builtPath} comes from an earlier client build and may be stale.` + advice,
            )
          }
          return
        } catch {
          // Not built yet.
        }
      }

      const sourcePath = resolve(config.root, 'index.html')
      try {
        indexHtmlContent = await readFile(sourcePath, 'utf-8')
        fallbackWarnings.set(
          name,
          `built client index.html not found${builtPath ? ` at ${builtPath}` : ''}; ` +
            `embedding the source ${sourcePath}, so SSR pages will load the source entry.` +
            (builder
              ? advice
              : ' Build the client first (`vite build --app`) or set ' +
                '`environments.client.build.outDir`.'),
        )
      } catch {
        // index.html not found, provide a minimal fallback
        indexHtmlContent =
          '<!DOCTYPE html><html><head></head><body><app-root></app-root></body></html>'
      }
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

        const appManifest = generateAppManifestCode({
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
  }
}

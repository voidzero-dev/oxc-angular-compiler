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
      if (this.environment?.config.consumer !== 'server') return

      // Like the Angular CLI, serve the built browser page, which loads the
      // hashed bundles, not the source one (@angular/build
      // src/utils/index-file/index-html-generator.ts builds index.server.html
      // from the augmented page).
      const config = this.environment.config
      if (clientIndexHtml) {
        const { source } = clientIndexHtml
        indexHtmlContent = typeof source === 'string' ? source : Buffer.from(source).toString()
        return
      }

      // A client built by an earlier run. buildStart runs before Vite empties outDir.
      const builder = !!this.environment.getTopLevelConfig?.().builder
      const clientOutDir = config.environments?.client?.build?.outDir
      const builtPath = clientOutDir && resolve(config.root, clientOutDir, 'index.html')
      if (builtPath) {
        try {
          indexHtmlContent = await readFile(builtPath, 'utf-8')
          if (builder) {
            this.warn(
              `the client is built after this server environment, so ${builtPath} ` +
                'comes from an earlier build and may be stale. List `client` first in ' +
                '`environments`.',
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
        this.warn(
          `built client index.html not found${builtPath ? ` at ${builtPath}` : ''}; ` +
            `embedding the source ${sourcePath}, so SSR pages will load the source entry. ` +
            (builder
              ? 'List `client` first in `environments`.'
              : 'Build the client first (`vite build --app`) or set ' +
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

        return {
          code: appManifest + engineManifest + code,
          map: null,
        }
      }
    },
  }
}

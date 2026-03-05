/**
 * Angular SSR Manifest Plugin
 *
 * Generates the Angular SSR manifests required by AngularNodeAppEngine.
 * Without these manifests, Angular throws:
 *   "Angular app engine manifest is not set."
 *
 * This plugin:
 * 1. Detects SSR builds (when Vite's build.ssr is true)
 * 2. Auto-injects manifest setup into files that use AngularNodeAppEngine/AngularAppEngine
 * 3. Provides the index.html as a server asset for SSR rendering
 *
 * @see https://github.com/voidzero-dev/oxc-angular-compiler/issues/60
 */

import { readFile } from 'node:fs/promises'
import { dirname, relative, resolve } from 'node:path'

import type { Plugin, ResolvedConfig } from 'vite'
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
  let isSSR = false
  let resolvedConfig: ResolvedConfig
  let ssrEntryPath: string
  let indexHtmlContent: string | undefined

  return {
    name: '@oxc-angular/vite-ssr-manifest',
    apply: 'build',

    config(userConfig) {
      isSSR = !!userConfig.build?.ssr
    },

    configResolved(config) {
      resolvedConfig = config

      if (!isSSR) return

      const workspaceRoot = config.root

      // Determine the SSR bootstrap entry (main.server.ts)
      ssrEntryPath = options.ssrEntry
        ? resolve(workspaceRoot, options.ssrEntry)
        : resolve(workspaceRoot, 'src/main.server.ts')
    },

    async buildStart() {
      if (!isSSR) return

      // Read index.html for the server asset
      const indexHtmlPath = resolve(resolvedConfig.root, 'index.html')
      try {
        indexHtmlContent = await readFile(indexHtmlPath, 'utf-8')
      } catch {
        // index.html not found, provide a minimal fallback
        indexHtmlContent =
          '<!DOCTYPE html><html><head></head><body><app-root></app-root></body></html>'
      }
    },

    transform(code, id) {
      if (!isSSR) return

      // Inject manifest setup into files that use AngularNodeAppEngine or AngularAppEngine
      // These are the SSR entry points that need the manifest before constructing the engine
      if (
        !id.includes('node_modules') &&
        !id.startsWith('\0') &&
        (code.includes('AngularNodeAppEngine') || code.includes('AngularAppEngine'))
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

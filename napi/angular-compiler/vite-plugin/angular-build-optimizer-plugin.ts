import { existsSync, readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'

import { optimizeAngularPackage } from '#binding'
import { createFilter, type Plugin } from 'vite'

type SideEffectsFilter = (id: string) => boolean

/**
 * Reads `sideEffects` from a file's nearest `package.json` and matches it like esbuild does for
 * the Angular CLI: a value that is not a boolean or an array counts as having side effects, and a
 * pattern with a slash only matches from the package root. Vite 8 builds (rolldown) differ on both.
 */
function createSideEffectsResolver(): SideEffectsFilter {
  const cache = new Map<string, SideEffectsFilter>()

  function filterForDir(dir: string): SideEffectsFilter {
    let filter = cache.get(dir)
    if (filter) {
      return filter
    }
    const pkgPath = join(dir, 'package.json')
    if (existsSync(pkgPath)) {
      let sideEffects: unknown
      try {
        const raw = readFileSync(pkgPath, 'utf-8')
        sideEffects = JSON.parse(raw.charCodeAt(0) === 0xfeff ? raw.slice(1) : raw).sideEffects
      } catch {
        // A package.json we cannot read is treated as having side effects.
      }
      if (typeof sideEffects === 'boolean') {
        filter = () => sideEffects
      } else if (Array.isArray(sideEffects)) {
        const patterns = sideEffects.filter((pattern) => typeof pattern === 'string')
        filter =
          patterns.length === 0
            ? () => false
            : createFilter(
                patterns.map((pattern: string) =>
                  pattern.includes('/') ? pattern : `**/${pattern}`,
                ),
                null,
                { resolve: dir },
              )
      } else {
        filter = () => true
      }
    } else {
      const parent = dirname(dir)
      filter = parent === dir ? () => true : filterForDir(parent)
    }
    cache.set(dir, filter)
    return filter
  }

  return (id) => filterForDir(dirname(id))(id)
}

/**
 * Build optimizer plugin for Angular production builds.
 *
 * This plugin:
 * 1. Sets up Angular-specific build defines (ngDevMode, ngJitMode, etc.)
 * 2. Applies the Angular build optimizer to pre-compiled packages from node_modules
 *
 * The optimizer enables better tree-shaking by:
 * - Removing Angular metadata calls (ɵsetClassMetadata, etc.)
 * - Wrapping static fields in pure IIFEs
 * - Adding /* @__PURE__ *\/ annotations to top-level calls of side-effect-free
 *   `@angular/*` packages
 * - Optimizing TypeScript enum patterns
 */
export function buildOptimizerPlugin({
  jit,
  sourcemap,
  thirdPartySourcemaps,
}: {
  jit: boolean
  sourcemap: boolean
  thirdPartySourcemaps: boolean
}): Plugin {
  let isProd = false
  const hasSideEffects = createSideEffectsResolver()

  return {
    name: '@oxc-angular/vite-optimizer',
    apply: 'build',
    config(userConfig) {
      isProd = userConfig.mode === 'production' || process.env['NODE_ENV'] === 'production'

      if (isProd) {
        return {
          define: {
            ngJitMode: jit ? 'true' : 'false',
            ngI18nClosureMode: 'false',
            ngDevMode: 'false',
          },
          oxc: {
            define: {
              ngDevMode: 'false',
              ngJitMode: jit ? 'true' : 'false',
              ngI18nClosureMode: 'false',
            },
          },
        }
      }

      return undefined
    },
    // `ngServerMode` is set per environment: one config can build the browser
    // and the server bundles (`vite build --app`, `builder.buildApp`), and
    // `vite build --ssr` builds the `ssr` environment. Like the Angular CLI, a
    // server bundle gets `true` and a production browser bundle `false`.
    configEnvironment(name, config) {
      // Vite's default when `consumer` is not set.
      const consumer = config.consumer ?? (name === 'client' ? 'client' : 'server')
      if (consumer === 'server') {
        return { define: { ngServerMode: 'true' } }
      }
      if (isProd) {
        return { define: { ngServerMode: 'false' } }
      }
      return undefined
    },
    transform: {
      filter: {
        // Match Angular FESM packages (e.g., @angular/core/fesm2022/core.mjs, @ngrx/store/fesm2022/ngrx-store.mjs)
        id: /fesm20.*\.[cm]?js$/,
      },
      async handler(code, id) {
        // Only optimize in production builds
        if (!isProd) {
          return
        }

        try {
          // Like the Angular CLI, only add top-level pure annotations to `@angular/*` packages
          // that declare themselves side-effect-free.
          const markPure = /[\\/]node_modules[\\/]@angular[\\/]/.test(id) && !hasSideEffects(id)
          const result = await optimizeAngularPackage(code, id, {
            sourcemap: sourcemap && thirdPartySourcemaps,
            elideMetadata: true,
            wrapStaticMembers: true,
            markPure,
            adjustEnums: true,
          })

          return {
            code: result.code,
            map: result.map ?? null,
          }
        } catch (e) {
          // If optimization fails, return the original code
          console.warn(`[angular-optimizer] Failed to optimize ${id}:`, e)
          return undefined
        }
      },
    },
  }
}

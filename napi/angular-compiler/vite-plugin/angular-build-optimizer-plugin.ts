import { optimizeAngularPackage } from '#binding'
import type { Plugin } from 'vite'

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
 * - Adding /* @__PURE__ *\/ annotations to top-level calls
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
            ngServerMode: `${!!userConfig.build?.ssr}`,
          },
          oxc: {
            define: {
              ngDevMode: 'false',
              ngJitMode: jit ? 'true' : 'false',
              ngI18nClosureMode: 'false',
              ngServerMode: `${!!userConfig.build?.ssr}`,
            },
          },
        }
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
          const result = await optimizeAngularPackage(code, id, {
            sourcemap: sourcemap && thirdPartySourcemaps,
            elideMetadata: true,
            wrapStaticMembers: true,
            markPure: true,
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

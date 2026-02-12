import { readFile } from 'node:fs/promises'

import { linkAngularPackageSync } from '#binding'
import type { Plugin } from 'vite'

/**
 * Angular Linker plugin for Vite.
 *
 * Processes pre-compiled Angular library code from node_modules that contains
 * partial compilation declarations (ɵɵngDeclare*). These declarations need to
 * be "linked" (converted to full ɵɵdefine* calls) at build time.
 *
 * Without this plugin, Angular falls back to JIT compilation which requires
 * @angular/compiler at runtime.
 *
 * Uses OXC's native Rust-based linker for fast, zero-dependency linking.
 *
 * This plugin works in two phases:
 * 1. During dependency optimization (Rolldown pre-bundling) via a Rolldown load plugin
 * 2. During Vite's transform pipeline for non-optimized node_modules files
 */

const LINKER_DECLARATION_PREFIX = '\u0275\u0275ngDeclare'

// Skip these packages - they don't need linking
const SKIP_REGEX = /[\\/]@angular[\\/](?:compiler|core)[\\/]/

// Match JS files in node_modules (Angular FESM bundles)
const NODE_MODULES_JS_REGEX = /node_modules\/.*\.[cm]?js$/

export function angularLinkerPlugin(): Plugin {
  return {
    name: '@voidzero-dev/vite-plugin-angular-linker',
    config() {
      return {
        optimizeDeps: {
          rolldownOptions: {
            plugins: [
              {
                name: 'angular-linker',
                load: {
                  filter: {
                    id: /\.[cm]?js$/,
                  },
                  async handler(id: string) {
                    // Skip @angular/compiler and @angular/core
                    if (SKIP_REGEX.test(id)) {
                      return
                    }

                    const code = await readFile(id, 'utf-8')

                    // Quick check: skip files without partial declarations
                    if (!code.includes(LINKER_DECLARATION_PREFIX)) {
                      return
                    }

                    const result = linkAngularPackageSync(code, id)

                    if (!result.linked) {
                      return
                    }

                    return result.code
                  },
                },
              },
            ],
          },
        },
      }
    },
    transform: {
      filter: {
        id: NODE_MODULES_JS_REGEX,
        code: LINKER_DECLARATION_PREFIX,
      },
      handler(code, id) {
        // Skip packages that don't need linking
        if (SKIP_REGEX.test(id)) {
          return
        }

        const result = linkAngularPackageSync(code, id)

        if (!result.linked) {
          return
        }

        return {
          code: result.code,
          map: result.map ?? null,
        }
      },
    },
  }
}

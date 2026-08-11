/**
 * PostCSS configuration for Bitwarden benchmark
 * Mirrors the configuration from bitwarden-clients/apps/web/postcss.config.js
 *
 * ESM: postcss-nested v8 is ESM-only.
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'

import autoprefixer from 'autoprefixer'
import postcssImport from 'postcss-import'
import postcssNested from 'postcss-nested'
import tailwindcss from 'tailwindcss'

const __dirname = path.dirname(fileURLToPath(import.meta.url))

// Paths relative to bitwarden-clients
const BITWARDEN_ROOT = path.resolve(__dirname, '../../../../../bitwarden-clients')
const BITWARDEN_WEB = path.resolve(BITWARDEN_ROOT, 'apps/web')
const BITWARDEN_LIBS = path.resolve(BITWARDEN_ROOT, 'libs')

export default {
  plugins: [
    // postcss-import for @import resolution
    postcssImport({
      path: [path.resolve(BITWARDEN_LIBS), path.resolve(BITWARDEN_WEB, 'src/scss')],
    }),

    // postcss-nested for SCSS-like nesting
    postcssNested,

    // Tailwind CSS
    tailwindcss({
      config: path.resolve(__dirname, 'tailwind.config.cjs'),
    }),

    // Autoprefixer for browser compatibility
    autoprefixer,
  ],
}

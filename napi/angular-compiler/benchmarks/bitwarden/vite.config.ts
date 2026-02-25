import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

// Use our local vite-plugin implementation
import { angular } from '@oxc-angular/vite/vite-plugin'
import { defineConfig, type UserConfig } from 'vite'
import wasm from 'vite-plugin-wasm'
import tsconfigPaths from 'vite-tsconfig-paths'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

// Paths to bitwarden-clients repository
// Adjust this path if your bitwarden-clients repo is in a different location
const BITWARDEN_ROOT = resolve(__dirname, '../../../../../bitwarden-clients')
const BITWARDEN_WEB = resolve(BITWARDEN_ROOT, 'apps/web')
const BITWARDEN_LIBS = resolve(BITWARDEN_ROOT, 'libs')

export default defineConfig(({ mode }) => {
  const isProd = mode === 'production'

  const config: UserConfig = {
    root: __dirname,

    plugins: [
      // WASM support for @bitwarden/sdk-internal
      wasm(),

      // Resolve TypeScript path aliases from bitwarden's tsconfig
      tsconfigPaths({
        root: BITWARDEN_ROOT,
        projects: [resolve(BITWARDEN_ROOT, 'tsconfig.base.json')],
      }),

      // OXC Angular compiler plugin
      angular({
        tsconfig: resolve(__dirname, 'tsconfig.app.json'),
        workspaceRoot: BITWARDEN_ROOT,
        inlineStylesExtension: 'scss',
        liveReload: !isProd,
        sourceMap: !isProd,
      }),
    ],

    // Resolve configuration
    resolve: {
      alias: {
        // Bitwarden-specific aliases that might not be in tsconfig
        '@bitwarden/web-vault': resolve(BITWARDEN_WEB, 'src'),
        // Node.js polyfills for browser
        path: 'path-browserify',
        buffer: 'buffer/',
      },
      extensions: ['.ts', '.js', '.mjs', '.json'],
      // Deduplicate packages to prevent multiple copies in the bundle
      dedupe: [
        // Angular packages - critical to prevent DI issues
        '@angular/core',
        '@angular/common',
        '@angular/compiler',
        '@angular/platform-browser',
        '@angular/platform-browser-dynamic',
        '@angular/router',
        '@angular/forms',
        '@angular/animations',
        '@angular/cdk',
        '@angular/material',
        // RxJS - commonly duplicated
        'rxjs',
        // Zone.js
        'zone.js',
        // Other common packages
        'tslib',
      ],
    },

    // CSS/SCSS configuration
    css: {
      preprocessorOptions: {
        scss: {
          // Include paths for SCSS imports
          loadPaths: [
            resolve(BITWARDEN_WEB, 'src/scss'),
            resolve(BITWARDEN_LIBS),
            resolve(BITWARDEN_ROOT, 'node_modules'),
          ],
          // Silence deprecation warnings from dependencies
          silenceDeprecations: ['legacy-js-api', 'import'],
        },
      },
      postcss: resolve(__dirname, 'postcss.config.cjs'),
    },

    // Define process.env variables that bitwarden uses
    define: {
      'process.env.NODE_ENV': JSON.stringify(mode),
      'process.env.ENV': JSON.stringify(mode === 'production' ? 'production' : 'development'),
      'process.env.APPLICATION_VERSION': JSON.stringify('0.0.0-benchmark'),
      'process.env.CACHE_TAG': JSON.stringify(Math.random().toString(36).substring(7)),
      'process.env.URLS': JSON.stringify({}),
      'process.env.STRIPE_KEY': JSON.stringify(''),
      'process.env.BRAINTREE_KEY': JSON.stringify(''),
      'process.env.PAYPAL_CONFIG': JSON.stringify({}),
      'process.env.FLAGS': JSON.stringify({}),
      'process.env.DEV_FLAGS': JSON.stringify({}),
      'process.env.ADDITIONAL_REGIONS': JSON.stringify([]),
      // Node.js polyfills
      global: 'globalThis',
    },

    // Build configuration
    build: {
      outDir: resolve(__dirname, 'dist'),
      sourcemap: true,
      minify: isProd,
      // Use ES2022 to avoid explicit resource management syntax issues
      target: 'es2022',
      // Rollup options
      rolldownOptions: {
        // Create shim variables for missing exports (type-only exports)
        shimMissingExports: true,
        input: {
          main: resolve(__dirname, 'index.html'),
        },
        output: {
          manualChunks(id) {
            // Group Angular packages into a vendor chunk
            if (id.includes('node_modules/@angular/')) {
              return 'vendor'
            }
            // Group RxJS into its own chunk
            if (id.includes('node_modules/rxjs')) {
              return 'rxjs'
            }
          },
          strictExecutionOrder: true,
        },
        // Handle type-only exports that get erased by TypeScript but Rollup still tries to resolve
        treeshake: {
          // Don't error on missing exports (type-only exports)
          moduleSideEffects: true,
        },
      },
      // Increase chunk size warning limit for large apps
      chunkSizeWarningLimit: 2000,
    },

    // Development server configuration
    server: {
      port: 4200,
      strictPort: false,
      open: false,
      // Proxy API requests if needed
      proxy: {
        '/api': {
          target: 'http://localhost:5000',
          changeOrigin: true,
          rewrite: (path) => path.replace(/^\/api/, ''),
        },
      },
    },

    oxc: {
      decorator: {
        emitDecoratorMetadata: true,
        legacy: true,
      },
      typescript: {
        onlyRemoveTypeImports: false,
      },
    },
    experimental: {
      enableNativePlugin: true,
      bundledDev: true,
    },
  }

  return config
})

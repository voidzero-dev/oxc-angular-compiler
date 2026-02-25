import { resolve, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'

// Use our local vite-plugin implementation
import { angular } from '@oxc-angular/vite/vite-plugin'
import { defineConfig, type UserConfig } from 'vite'
import tsconfigPaths from 'vite-tsconfig-paths'

const __filename = fileURLToPath(import.meta.url)
const __dirname = dirname(__filename)

// Paths to typedb-web repository
// Adjust this path if your typedb-web repo is in a different location
const TYPEDB_ROOT = resolve(__dirname, '../../../../../typedb-web')
const TYPEDB_MAIN = resolve(TYPEDB_ROOT, 'main')
const TYPEDB_COMMON = resolve(TYPEDB_ROOT, 'common')
const TYPEDB_SCHEMA = resolve(TYPEDB_ROOT, 'schema')

export default defineConfig(({ mode }) => {
  const isProd = mode === 'production'

  const config: UserConfig = {
    root: __dirname,

    plugins: [
      // Resolve TypeScript path aliases from typedb-web's tsconfig
      tsconfigPaths({
        root: TYPEDB_MAIN,
        projects: [resolve(TYPEDB_MAIN, 'tsconfig.json')],
      }),

      // OXC Angular compiler plugin
      angular({
        tsconfig: resolve(__dirname, 'tsconfig.app.json'),
        workspaceRoot: TYPEDB_ROOT,
        inlineStylesExtension: 'scss',
        liveReload: !isProd,
        sourceMap: !isProd,
      }),
    ],

    // Resolve configuration
    resolve: {
      alias: {
        // Workspace package aliases - point to source for proper ESM resolution
        'typedb-web-common/lib': resolve(TYPEDB_COMMON, 'src/scripts'),
        'typedb-web-common': TYPEDB_COMMON,
        'typedb-web-schema': TYPEDB_SCHEMA,
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
            resolve(TYPEDB_MAIN, 'src/styles'),
            resolve(TYPEDB_COMMON, 'src/styles'),
            resolve(TYPEDB_MAIN, 'node_modules'),
            resolve(TYPEDB_ROOT, 'node_modules'),
          ],
          // Silence deprecation warnings from dependencies
          silenceDeprecations: ['legacy-js-api', 'import'],
        },
      },
    },

    // Define environment variables
    define: {
      'process.env.NODE_ENV': JSON.stringify(mode),
      global: 'globalThis',
    },

    // Build configuration
    build: {
      outDir: resolve(__dirname, 'dist'),
      sourcemap: true,
      minify: isProd,
      target: 'es2022',
      rolldownOptions: {
        shimMissingExports: true,
        input: {
          main: resolve(__dirname, 'index.html'),
        },
        output: {
          manualChunks(id) {
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
      chunkSizeWarningLimit: 2000,
    },

    // Development server configuration
    server: {
      port: 4201,
      strictPort: false,
      open: false,
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

/**
 * Oxc Angular Vite Plugin
 *
 * A simplified Vite plugin for Angular that uses Oxc's Rust-based compiler.
 * This plugin handles:
 * - Template compilation
 * - Style processing
 * - Hot Module Replacement (HMR)
 */

import { watch } from 'node:fs'
import { readFile } from 'node:fs/promises'
import { ServerResponse } from 'node:http'
import { dirname, resolve } from 'node:path'

import { createDebug } from 'obug'
import type { Plugin, ResolvedConfig, ViteDevServer, Connect } from 'vite'
import { preprocessCSS, normalizePath } from 'vite'

// Debug loggers - enable with DEBUG=vite:oxc-angular:*
const debugHmr = createDebug('vite:oxc-angular:hmr')
const debugTransform = createDebug('vite:oxc-angular:transform')

import {
  transformAngularFile,
  extractComponentUrls,
  encapsulateStyle,
  compileForHmrSync,
  type TransformOptions,
  type ResolvedResources,
  type AngularVersion,
} from '#binding'

import { buildOptimizerPlugin } from './angular-build-optimizer-plugin.js'
import { jitPlugin } from './angular-jit-plugin.js'
import { angularLinkerPlugin } from './angular-linker-plugin.js'
import { ssrManifestPlugin } from './angular-ssr-manifest-plugin.js'

/**
 * Plugin options for the Angular Vite plugin.
 */
export interface PluginOptions {
  /** Path to tsconfig.json (used for file discovery, not TypeScript compilation). */
  tsconfig?: string

  /** Workspace root directory. */
  workspaceRoot?: string

  /** Extension for inline styles (css, scss, etc.). */
  inlineStylesExtension?: string

  /** Enable JIT compilation mode. */
  jit?: boolean

  /** Enable live reload / HMR. */
  liveReload?: boolean

  /** Enable source map generation. */
  sourceMap?: boolean | { scripts?: boolean; vendor?: boolean }

  /** Enable zoneless mode. */
  zoneless?: boolean

  /** File replacements (for environment files). */
  fileReplacements?: Array<{ replace: string; with: string }>

  /** Path to main.server.ts for SSR manifest generation. Auto-detected from src/main.server.ts if not specified. */
  ssrEntry?: string

  /**
   * Angular version to target.
   *
   * Controls which runtime instructions are emitted. For example, Angular 19
   * uses `ɵɵtemplate` for `@if`/`@switch` blocks, while Angular 20+ uses
   * `ɵɵconditionalCreate`/`ɵɵconditionalBranchCreate`.
   *
   * When not set, assumes latest Angular version (v20+ behavior).
   *
   * @example
   * ```ts
   * angular({ angularVersion: { major: 19, minor: 0, patch: 0 } })
   * ```
   */
  angularVersion?: AngularVersion
}

// Match all TypeScript files - we'll filter by @Component/@Directive decorator in the handler
const ANGULAR_TS_REGEX = /\.tsx?$/
const ANGULAR_COMPONENT_PREFIX = '@ng/component'

/**
 * Create the Angular Vite plugin.
 */
export function angular(options: PluginOptions = {}): Plugin[] {
  const workspaceRoot = options.workspaceRoot ?? process.cwd()

  // Process file replacements
  let fileReplacements: Record<string, string> | undefined
  if (options.fileReplacements) {
    fileReplacements = {}
    for (const replacement of options.fileReplacements) {
      const from = resolve(workspaceRoot, replacement.replace)
      const to = resolve(workspaceRoot, replacement.with)
      fileReplacements[from] = to
    }
  }

  // Resolve options
  const pluginOptions = {
    workspaceRoot,
    inlineStylesExtension: options.inlineStylesExtension ?? 'css',
    jit: options.jit ?? false,
    liveReload: options.liveReload ?? true,
    sourceMap:
      typeof options.sourceMap === 'boolean'
        ? options.sourceMap
        : (options.sourceMap?.scripts ?? true),
    zoneless: options.zoneless ?? false,
    fileReplacements,
    angularVersion: options.angularVersion,
  }

  let resolvedConfig: ResolvedConfig
  let viteServer: ViteDevServer | undefined
  let watchMode = false

  // Track component IDs for HMR
  const componentIds = new Map<string, string>()

  // Reverse mapping: resource file path → component file path
  const resourceToComponent = new Map<string, string>()

  // Cache for resolved resources
  const resourceCache = new Map<string, string>()

  // Track component files with pending HMR updates (set by fs.watch, checked by HMR endpoint)
  const pendingHmrUpdates = new Set<string>()

  /**
   * Resolve external template/style URLs and read their contents.
   */
  async function resolveResources(
    code: string,
    id: string,
  ): Promise<{ resources: ResolvedResources; dependencies: string[] }> {
    const { templateUrls, styleUrls } = await extractComponentUrls(code, id)
    const dir = dirname(id)
    const dependencies: string[] = []

    // NAPI-RS expects plain objects for HashMap, not JavaScript Maps
    const templates: Record<string, string> = {}
    const styles: Record<string, string[]> = {}

    // Resolve templates
    for (const templateUrl of templateUrls) {
      const templatePath = resolve(dir, templateUrl)
      dependencies.push(templatePath)

      let content = resourceCache.get(templatePath)
      if (!content) {
        try {
          content = await readFile(templatePath, 'utf-8')
          resourceCache.set(templatePath, content)
        } catch {
          console.warn(`Failed to read template: ${templatePath}`)
          continue
        }
      }
      templates[templateUrl] = content
    }

    // Resolve styles
    for (const styleUrl of styleUrls) {
      const stylePath = resolve(dir, styleUrl)
      dependencies.push(stylePath)

      let content = resourceCache.get(stylePath)
      if (!content) {
        try {
          content = await readFile(stylePath, 'utf-8')
          // Preprocess styles (SCSS, Less, etc.)
          if (resolvedConfig) {
            try {
              const processed = await preprocessCSS(content, stylePath, resolvedConfig as any)
              content = processed.code
            } catch (e) {
              console.warn(`Failed to preprocess style: ${stylePath}`, e)
            }
          }
          resourceCache.set(stylePath, content)
        } catch {
          console.warn(`Failed to read style: ${stylePath}`)
          continue
        }
      }
      styles[styleUrl] = [content]
    }

    // Note: NAPI-RS HashMap binds to plain objects at runtime, despite Map type in index.d.ts
    return { resources: { templates, styles } as unknown as ResolvedResources, dependencies }
  }

  /**
   * Main Angular plugin for file transformation.
   */
  function angularPlugin(): Plugin {
    return {
      name: '@oxc-angular/vite',
      async config(_, { command }) {
        watchMode = command === 'serve'

        return {
          optimizeDeps: {
            include: ['rxjs/operators', 'rxjs'],
            exclude: ['@angular/platform-server'],
          },
          ...(options.tsconfig && {
            build: {
              rolldownOptions: {
                tsconfig: options.tsconfig,
              },
            },
          }),
        }
      },
      configResolved(config) {
        resolvedConfig = config
      },
      configureServer(server) {
        viteServer = server

        // Track watched template files
        const watchedTemplates = new Set<string>()

        // Use fs.watch for template files instead of Vite's watcher
        // This bypasses Vite's internal handling which causes full reloads
        const watchTemplateFile = (file: string) => {
          if (watchedTemplates.has(file)) return
          watchedTemplates.add(file)

          // Dynamically unwatch from Vite's watcher - this is more precise than static glob patterns
          // and handles any file naming convention (app.css, app.component.css, styles.scss, etc.)
          server.watcher.unwatch(file)
          debugHmr('unwatched from Vite, adding custom watch: %s', file)

          watch(file, { persistent: true }, async (eventType) => {
            if (eventType === 'change') {
              const normalizedFile = normalizePath(file)
              debugHmr('resource file change: %s', normalizedFile)

              // Invalidate resource cache
              resourceCache.delete(normalizedFile)

              // Handle template/style file changes for HMR
              if (pluginOptions.liveReload) {
                const componentFile = resourceToComponent.get(normalizedFile)
                if (componentFile && componentIds.has(componentFile)) {
                  debugHmr('resource change triggers HMR: %s -> %s', normalizedFile, componentFile)

                  // Mark this component as having a pending HMR update so the
                  // HMR endpoint serves the update module instead of an empty response.
                  pendingHmrUpdates.add(componentFile)

                  // Send HMR update event
                  const componentId = `${componentFile}@${componentIds.get(componentFile)}`
                  const encodedId = encodeURIComponent(componentId)
                  debugHmr('sending WS event: id=%s', encodedId)
                  // Vite expects { type: "custom", event, data } format for custom HMR events
                  const eventData = { id: encodedId, timestamp: Date.now() }
                  server.ws.send({
                    type: 'custom',
                    event: 'angular:component-update',
                    data: eventData,
                  })
                }
              }
            }
            debugHmr('added custom fs.watch for resource: %s', file)
          })
        }

        // Expose the function so transform can call it
        ;(server as any).__angularWatchTemplate = watchTemplateFile

        // Listen for angular:invalidate events from client
        // When Angular's runtime HMR update fails, it sends this event to trigger a full reload
        server.ws.on(
          'angular:invalidate',
          (data: { id: string; message: string; error: boolean }) => {
            console.warn(`[Angular HMR] Runtime update failed for ${data.id}: ${data.message}`)
            server.ws.send({
              type: 'full-reload',
              path: '*',
            })
          },
        )

        // HMR component update endpoint
        if (pluginOptions.liveReload) {
          const angularComponentMiddleware: Connect.HandleFunction = async (
            req: Connect.IncomingMessage,
            res: ServerResponse<Connect.IncomingMessage>,
            next: Connect.NextFunction,
          ) => {
            if (!req.url?.includes(ANGULAR_COMPONENT_PREFIX)) {
              next()
              return
            }

            const requestUrl = new URL(req.url, 'http://localhost')
            const componentId = requestUrl.searchParams.get('c')

            if (!componentId) {
              res.statusCode = 400
              res.end()
              return
            }

            const decodedComponentId = decodeURIComponent(componentId)
            const atIndex = decodedComponentId.indexOf('@')

            // Validate component ID format: should be "filePath@ClassName"
            if (atIndex === -1) {
              console.error(`[Angular HMR] Invalid component ID format: ${componentId}`)
              res.statusCode = 400
              res.end()
              return
            }

            const fileId = decodedComponentId.slice(0, atIndex)
            const resolvedId = resolve(process.cwd(), fileId)

            // Only return HMR update module if there's a pending update from our
            // custom fs.watch handler. On initial page load, there are no pending
            // updates, so we return an empty response. This prevents ɵɵreplaceMetadata
            // from being called unnecessarily during initial load, which would
            // re-create views and cause errors with @Required() decorators.
            if (!pendingHmrUpdates.has(fileId)) {
              res.setHeader('Content-Type', 'text/javascript')
              res.setHeader('Cache-Control', 'no-cache')
              res.end('')
              return
            }
            pendingHmrUpdates.delete(fileId)

            try {
              const source = await readFile(resolvedId, 'utf-8')
              const { templateUrls, styleUrls } = await extractComponentUrls(source, resolvedId)
              const dir = dirname(resolvedId)

              // Read fresh template content (bypass cache for HMR)
              let templateContent: string | null = null
              if (templateUrls.length > 0) {
                const templatePath = resolve(dir, templateUrls[0])
                templateContent = await readFile(templatePath, 'utf-8')
              } else {
                templateContent = extractInlineTemplate(source)
              }

              if (templateContent) {
                const className = componentIds.get(resolvedId) ?? 'Component'

                // Read fresh style content for all style URLs
                let styles: string[] | null = null
                if (styleUrls.length > 0) {
                  const styleContents: string[] = []
                  for (const styleUrl of styleUrls) {
                    const stylePath = resolve(dir, styleUrl)
                    try {
                      let styleContent = await readFile(stylePath, 'utf-8')
                      if (resolvedConfig) {
                        const processed = await preprocessCSS(
                          styleContent,
                          stylePath,
                          resolvedConfig as any,
                        )
                        styleContent = processed.code
                      }
                      styleContents.push(styleContent)
                    } catch {
                      // Style file not found, continue without this style
                    }
                  }
                  if (styleContents.length > 0) {
                    styles = styleContents
                  }
                }

                const result = compileForHmrSync(templateContent, className, resolvedId, styles)

                res.setHeader('Content-Type', 'text/javascript')
                res.setHeader('Cache-Control', 'no-cache')
                res.end(result.hmrModule)
                return
              }
            } catch (e) {
              const error = e as Error
              const errorMessage = error.message + (error.stack ? '\n' + error.stack : '')
              console.error('[Angular HMR] Update failed:', errorMessage)

              // Send angular:invalidate event to trigger graceful full reload
              // This matches Angular's HMR error fallback pattern
              server.ws.send({
                type: 'custom',
                event: 'angular:invalidate',
                data: { id: componentId, message: errorMessage, error: true },
              })

              res.setHeader('Content-Type', 'text/javascript')
              res.setHeader('Cache-Control', 'no-cache')
              res.end('')
              return
            }

            // No template content found
            res.setHeader('Content-Type', 'text/javascript')
            res.setHeader('Cache-Control', 'no-cache')
            res.end('')
          }

          server.middlewares.use(angularComponentMiddleware)
        }
      },
      transform: {
        order: 'pre',
        filter: {
          id: ANGULAR_TS_REGEX,
        },
        async handler(code, id) {
          // Skip node_modules
          if (id.includes('node_modules')) {
            return
          }

          // Quick check for Angular decorators - avoids parsing files without them
          // OXC handles @Component, @Directive, @NgModule, @Injectable, and @Pipe
          const hasAngularDecorator =
            code.includes('@Component') ||
            code.includes('@Directive') ||
            code.includes('@NgModule') ||
            code.includes('@Injectable') ||
            code.includes('@Pipe')
          if (!hasAngularDecorator) {
            return
          }

          // Apply file replacements
          const actualId = pluginOptions.fileReplacements?.[id] ?? id

          // Resolve external resources
          const { resources, dependencies } = await resolveResources(code, actualId)

          // Track dependencies for HMR
          // DON'T use addWatchFile - it creates modules in Vite's graph!
          // Instead, use our custom watcher that doesn't create modules.
          if (watchMode && viteServer) {
            const watchFn = (viteServer as any).__angularWatchTemplate
            for (const dep of dependencies) {
              const normalizedDep = normalizePath(dep)
              // Track reverse mapping for HMR: resource → component
              resourceToComponent.set(normalizedDep, actualId)
              // Add to our custom watcher
              if (watchFn) {
                watchFn(normalizedDep)
              }
            }
          }

          // Transform with Rust compiler
          const transformOptions: TransformOptions = {
            sourcemap: pluginOptions.sourceMap,
            jit: pluginOptions.jit,
            hmr: pluginOptions.liveReload && watchMode,
            angularVersion: pluginOptions.angularVersion,
          }

          const result = await transformAngularFile(code, actualId, transformOptions, resources)

          // Report errors and warnings
          for (const error of result.errors) {
            this.error(error.message)
          }
          for (const warning of result.warnings) {
            this.warn(warning.message)
          }

          // Track component IDs for HMR
          if (pluginOptions.liveReload) {
            // templateUpdates is a plain object (NAPI HashMap → JS object)
            const templateUpdateKeys = Object.keys(result.templateUpdates)
            debugTransform(
              'transform %s templateUpdates=%O deps=%O',
              actualId,
              templateUpdateKeys,
              dependencies,
            )
            for (const componentId of templateUpdateKeys) {
              const [, className] = componentId.split('@')
              componentIds.set(actualId, className)
              debugHmr('registered: %s -> %s', actualId, className)
            }
          }

          return {
            code: result.code,
            map: result.map ?? null,
          }
        },
      },
      async handleHotUpdate(ctx) {
        if (!pluginOptions.liveReload) return

        debugHmr('handleHotUpdate file=%s', ctx.file)
        debugHmr(
          'ctx.modules=%d ids=%s',
          ctx.modules.length,
          ctx.modules.map((m) => m.id).join(', '),
        )

        // Template/style files are handled by our custom fs.watch in configureServer.
        // We dynamically unwatch them from Vite's watcher during transform, so they shouldn't
        // normally trigger handleHotUpdate. If they do appear here (e.g., file not yet transformed
        // or from another plugin), return [] to prevent Vite's default handling.
        if (/\.(html?|css|scss|sass|less)$/.test(ctx.file)) {
          debugHmr('ignoring resource file in handleHotUpdate (handled by custom watcher)')
          return []
        }

        // Handle component file changes
        const isComponent = ANGULAR_TS_REGEX.test(ctx.file)
        const hasComponentId = componentIds.has(ctx.file)
        debugHmr(
          'component check: isComponent=%s hasComponentId=%s file=%s',
          isComponent,
          hasComponentId,
          ctx.file,
        )
        debugHmr('componentIds keys: %O', Array.from(componentIds.keys()))

        if (isComponent && hasComponentId) {
          debugHmr('triggering full reload for component file change')
          // Component FILE changes require a full reload because:
          // - Class definition changes can't be hot-swapped safely
          // - Constructor, methods, signals, and state changes need a fresh start
          // - Only template/style changes support HMR (handled by fs.watch separately)
          //
          // This matches Angular's official behavior - they only support HMR for
          // template and style changes, not component class changes.

          // Invalidate the component module
          const componentModule = ctx.server.moduleGraph.getModuleById(ctx.file)
          if (componentModule) {
            ctx.server.moduleGraph.invalidateModule(componentModule)
          }

          // Clear any cached resources
          resourceCache.delete(normalizePath(ctx.file))

          // Trigger full reload
          debugHmr('sending full-reload WebSocket message for %s', ctx.file)
          ctx.server.ws.send({
            type: 'full-reload',
            path: ctx.file,
          })
          debugHmr('full-reload message sent')

          return []
        }

        return ctx.modules
      },
    }
  }

  /**
   * Plugin to encapsulate component styles.
   */
  function stylesPlugin(): Plugin {
    return {
      name: '@oxc-angular/vite-styles',
      transform: {
        filter: {
          id: /ngcomp/,
        },
        handler(code, id) {
          if (!pluginOptions.liveReload) return

          const params = new URL(id, 'http://localhost').searchParams
          const componentId = params.get('ngcomp')
          const encapsulation = params.get('e')

          // Only encapsulate for emulated encapsulation (e=0)
          if (encapsulation === '0' && componentId) {
            const encapsulated = encapsulateStyle(code, componentId)
            return {
              code: encapsulated,
              map: null,
            }
          }

          return undefined
        },
      },
    }
  }

  return [
    angularPlugin(),
    stylesPlugin(),
    angularLinkerPlugin(),
    pluginOptions.jit &&
      jitPlugin({
        inlineStylesExtension: pluginOptions.inlineStylesExtension,
      }),
    buildOptimizerPlugin({
      jit: pluginOptions.jit,
      sourcemap: pluginOptions.sourceMap,
      thirdPartySourcemaps: false,
    }),
    ssrManifestPlugin({
      ssrEntry: options.ssrEntry,
    }),
  ].filter(Boolean) as Plugin[]
}

/**
 * Extract inline template from @Component decorator.
 */
function extractInlineTemplate(code: string): string | null {
  // Simple regex to extract inline template
  const templateMatch = code.match(/template\s*:\s*`([^`]*)`/s)
  if (templateMatch) {
    return templateMatch[1]
  }

  const templateQuoteMatch = code.match(/template\s*:\s*['"]([^'"]*)['"]/)
  if (templateQuoteMatch) {
    return templateQuoteMatch[1]
  }

  return null
}

export { angular as default }

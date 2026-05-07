/**
 * Oxc Angular Vite Plugin
 *
 * A simplified Vite plugin for Angular that uses Oxc's Rust-based compiler.
 * This plugin handles:
 * - Template compilation
 * - Style processing
 * - Hot Module Replacement (HMR)
 */

import { readFileSync } from 'node:fs'
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

  /**
   * Minify final component styles before emitting them into `styles: [...]`.
   *
   * When set to `"auto"` or left undefined, this follows Vite's resolved CSS
   * minification settings for production builds:
   *
   * - `true`: always minify component styles
   * - `false`: never minify component styles
   * - `"auto"`/`undefined`: use `build.cssMinify` when set, otherwise fall back
   *   to `build.minify`
   *
   * In dev, `"auto"` defaults to `false`.
   */
  minifyComponentStyles?: boolean | 'auto'

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

  /** Optional callback to transform template content before compilation. Applied during both initial build and HMR. */
  templateTransform?: (content: string, filePath: string) => string
}

// Match all TypeScript files - we'll filter by @Component/@Directive decorator in the handler
const ANGULAR_TS_REGEX = /\.tsx?$/
const ANGULAR_COMPONENT_PREFIX = '@ng/component'
type InlineBuildMinifyOptions = {
  cssMinify?: boolean | string
  minify?: boolean | string
}

function resolveMinifyComponentStyles(
  option: PluginOptions['minifyComponentStyles'],
  isBuild: boolean,
  inlineBuild?: InlineBuildMinifyOptions,
  outputMinify?: unknown,
  resolvedBuild?: ResolvedConfig['build'],
): boolean {
  if (typeof option === 'boolean') {
    return option
  }

  if (!isBuild) {
    return false
  }

  if (inlineBuild?.cssMinify !== undefined) {
    return inlineBuild.cssMinify !== false
  }

  if (inlineBuild?.minify !== undefined) {
    return inlineBuild.minify !== false
  }

  if (outputMinify !== undefined) {
    return outputMinify !== false
  }

  if (resolvedBuild?.cssMinify !== undefined) {
    return resolvedBuild.cssMinify !== false
  }

  return resolvedBuild?.minify !== false
}

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
  let inlineBuild: InlineBuildMinifyOptions | undefined
  let outputMinify: unknown

  // Track component IDs for HMR
  const componentIds = new Map<string, string>()

  // Reverse mapping: resource file path → component file path
  const resourceToComponent = new Map<string, string>()

  // Cache for resolved resources
  const resourceCache = new Map<string, string>()

  // Component files queued for HMR delivery. Populated by `handleHotUpdate`
  // when an external resource or inline template/style change is detected,
  // and consumed by the `@ng/component` HTTP endpoint, which reads it to
  // decide whether to serve the update module or an empty response.
  const pendingHmrUpdates = new Set<string>()

  // Cache inline template content per .ts file for detecting template-only changes
  const inlineTemplateCache = new Map<string, string>()

  // Cache the inline styles array per .ts file. Used by the HMR endpoint to
  // serve fresh inline styles to ɵɵreplaceMetadata when the styles change.
  const inlineStylesCache = new Map<string, string[]>()

  // Cache the source of each component .ts file with its `template:` and
  // `styles:` decorator fields stripped. If the stripped form is byte-identical
  // before and after a save, we know only the template / styles changed and
  // can dispatch an HMR update instead of a full reload.
  const componentMetadataCache = new Map<string, string>()

  function getMinifyComponentStyles(context?: {
    environment?: { config?: { build?: ResolvedConfig['build'] } }
  }): boolean {
    return resolveMinifyComponentStyles(
      options.minifyComponentStyles,
      !watchMode,
      inlineBuild,
      outputMinify,
      context?.environment?.config?.build ?? resolvedConfig?.build,
    )
  }

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
          if (options.templateTransform) {
            content = options.templateTransform(content, templatePath)
          }
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
      async config(config, { command }) {
        watchMode = command === 'serve'
        inlineBuild = config.build
          ? {
              cssMinify: config.build.cssMinify,
              minify: config.build.minify,
            }
          : undefined

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
      outputOptions(options) {
        outputMinify = options.minify
        return null
      },
      // Safety net: resolve @ng/component virtual modules in SSR context.
      // The browser serves these via HTTP middleware, but Vite's module runner
      // (used by Nitro/SSR) resolves through plugin hooks instead.
      resolveId(source, _importer, options) {
        if (options?.ssr && source.includes(ANGULAR_COMPONENT_PREFIX)) {
          // Return as virtual module (with \0 prefix per Vite convention)
          return `\0${source}`
        }
      },
      load(id, options) {
        if (options?.ssr && id.startsWith('\0') && id.includes(ANGULAR_COMPONENT_PREFIX)) {
          // Return empty module — SSR doesn't need HMR update modules
          return 'export default undefined;'
        }
      },
      configureServer(server) {
        viteServer = server

        // No custom file watcher — Vite's chokidar already watches every file
        // it knows about, and `transform()` registers component templates and
        // styles in `resourceToComponent` so they end up in Vite's module
        // graph. All FS-event dispatch happens in `handleHotUpdate` below.
        //
        // Earlier versions of this plugin used `node:fs.watch(file, …)` per
        // resource and called `server.watcher.unwatch(file)` to suppress
        // Vite's default behavior. That setup misses single-`writeFile` events
        // on macOS (the AI-tool/IDE pattern that hits FSEvents coalescing
        // bugs) and silently drops 'rename' events from atomic-rename saves
        // (vim, IntelliJ). The `handleHotUpdate` hook is the canonical Vite
        // plugin extension point for what we need; using it lets Vite's
        // single watcher do its job, simplifying the plugin and matching how
        // Angular CLI's `@angular/build` esbuild dev server is structured.

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

            // Only return an HMR update module if `handleHotUpdate` queued
            // one for this component. On initial page load there are no
            // pending updates, so we return an empty response. This prevents
            // ɵɵreplaceMetadata from being called unnecessarily during
            // initial load, which would re-create views and cause errors
            // with @Required() decorators.
            if (!pendingHmrUpdates.has(fileId)) {
              res.setHeader('Content-Type', 'text/javascript')
              res.setHeader('Cache-Control', 'no-cache')
              res.end('')
              return
            }

            try {
              const source = await readFile(resolvedId, 'utf-8')
              const { templateUrls, styleUrls } = await extractComponentUrls(source, resolvedId)
              const dir = dirname(resolvedId)

              // Read fresh template content (bypass cache for HMR)
              let templateContent: string | null = null
              if (templateUrls.length > 0) {
                const templatePath = resolve(dir, templateUrls[0])
                templateContent = await readFile(templatePath, 'utf-8')
                if (options.templateTransform) {
                  templateContent = options.templateTransform(templateContent, templatePath)
                }
              } else {
                templateContent = extractInlineTemplate(source)
              }

              if (templateContent) {
                const className = componentIds.get(resolvedId) ?? 'Component'

                // Read fresh style content. External styleUrls are read from
                // disk and run through Vite's preprocessCSS (so SCSS/LESS
                // resolve correctly); inline styles are extracted from the
                // .ts source as plain CSS strings.
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
                } else {
                  // No external styleUrls — fall back to inline `styles: […]`.
                  const inlineStyles = extractInlineStyles(source)
                  if (inlineStyles !== null && inlineStyles.length > 0) {
                    styles = inlineStyles
                  }
                }

                const result = compileForHmrSync(templateContent, className, resolvedId, styles, {
                  angularVersion: pluginOptions.angularVersion,
                  minifyComponentStyles: getMinifyComponentStyles(),
                })

                // Only consume the pending slot once we have real content to
                // serve. If we deleted unconditionally and the file was
                // transiently empty (truncate phase of an atomic write on
                // Linux), the next inotify event's request would find no
                // pending entry and deliver no HMR.
                pendingHmrUpdates.delete(fileId)
                res.setHeader('Content-Type', 'text/javascript')
                res.setHeader('Cache-Control', 'no-cache')
                res.end(result.hmrModule)
                return
              }
            } catch (e) {
              const error = e as Error
              const errorMessage = error.message + (error.stack ? '\n' + error.stack : '')
              console.error('[Angular HMR] Update failed:', errorMessage)

              // Consume the pending slot on error to prevent repeated failed
              // compilations on every subsequent browser request.
              pendingHmrUpdates.delete(fileId)

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

            // Template content was empty or null — either the file is in a
            // transient state during a multi-step write (truncate phase), or
            // the template was legitimately removed. In both cases, preserve
            // the pending entry: a transient empty resolves on the next watcher
            // event; a permanent removal is bounded — the next successful save
            // will consume the entry.
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
        async handler(code, id, options) {
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

          // Disable HMR for SSR transforms. SSR bundles must not contain HMR
          // initialization code that dynamically imports @ng/component virtual
          // modules, as those are served via HTTP middleware only. This matches
          // Angular's official behavior where _enableHmr is only set for browser
          // bundles (see @angular/build application-code-bundle.js).
          const isSSR = !!options?.ssr

          // Track dependencies for resource cache invalidation and HMR.
          // We don't call addWatchFile (which would create modules in Vite's
          // graph) or maintain a custom watcher — Vite's chokidar already
          // sees these files via its normal HMR pipeline, and our
          // `handleHotUpdate` hook below dispatches based on
          // `resourceToComponent` membership.
          if (watchMode && viteServer) {
            // Prune stale reverse mappings: if this component previously
            // referenced different resources (e.g., templateUrl was renamed),
            // drop the old entries so `handleHotUpdate` stops treating them
            // as component-owned.
            const newDeps = new Set(dependencies.map(normalizePath))
            for (const [resource, owner] of resourceToComponent) {
              if (owner === actualId && !newDeps.has(resource)) {
                resourceToComponent.delete(resource)
              }
            }

            for (const dep of dependencies) {
              const normalizedDep = normalizePath(dep)
              // Track reverse mapping for HMR: resource → component
              resourceToComponent.set(normalizedDep, actualId)
            }
          }

          // Transform with Rust compiler
          const transformOptions: TransformOptions = {
            sourcemap: pluginOptions.sourceMap,
            jit: pluginOptions.jit,
            hmr: pluginOptions.liveReload && watchMode && !isSSR,
            angularVersion: pluginOptions.angularVersion,
            minifyComponentStyles: getMinifyComponentStyles(this as any),
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

            // Cache inline template / styles for detecting template-only or
            // styles-only changes in handleHotUpdate, and the metadata-stripped
            // source for cheaply diffing whether anything else changed.
            const inlineTemplate = extractInlineTemplate(code)
            if (inlineTemplate !== null) {
              inlineTemplateCache.set(actualId, inlineTemplate)
            }
            const inlineStyles = extractInlineStyles(code)
            if (inlineStyles !== null) {
              inlineStylesCache.set(actualId, inlineStyles)
            } else {
              inlineStylesCache.delete(actualId)
            }
            componentMetadataCache.set(actualId, stripComponentMetadata(code))
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

        const normalizedFile = normalizePath(ctx.file)

        // Helper: dispatch a component HMR update for the owning component
        // (used by both external resource and inline template/style branches).
        // Returns true if dispatched.
        const dispatchComponentUpdate = (componentFile: string): boolean => {
          if (!componentIds.has(componentFile)) return false
          // The HMR HTTP endpoint reads this set to decide whether to serve
          // the update module or an empty response.
          pendingHmrUpdates.add(componentFile)

          // Invalidate the component's module so the next request reads fresh
          // template/style content.
          const mod = ctx.server.moduleGraph.getModuleById(componentFile)
          if (mod) ctx.server.moduleGraph.invalidateModule(mod)

          const componentId = `${componentFile}@${componentIds.get(componentFile)}`
          const encodedId = encodeURIComponent(componentId)
          debugHmr('sending angular:component-update id=%s', encodedId)
          ctx.server.ws.send({
            type: 'custom',
            event: 'angular:component-update',
            data: { id: encodedId, timestamp: Date.now() },
          })
          return true
        }

        // ------------------------------------------------------------
        // Branch 1: external component resource (templateUrl / styleUrl)
        // ------------------------------------------------------------
        // Files like `foo.component.html` or `foo.component.scss` referenced
        // by a component get HMR — Angular's runtime hot-swaps templates and
        // styles without re-instantiating the component. Non-component
        // resources (e.g. global stylesheets in main.ts) fall through to
        // Vite's default CSS HMR pipeline so PostCSS/Tailwind etc. still
        // process them.
        if (/\.(html?|css|scss|sass|less)$/.test(ctx.file)) {
          if (resourceToComponent.has(normalizedFile)) {
            const componentFile = resourceToComponent.get(normalizedFile)!
            resourceCache.delete(normalizedFile)
            if (dispatchComponentUpdate(componentFile)) {
              debugHmr('external resource HMR: %s -> %s', normalizedFile, componentFile)
              return []
            }
          }
          // Not a tracked component resource — let Vite handle it.
          return ctx.modules
        }

        // ------------------------------------------------------------
        // Branch 2: component .ts (has @Component decorator)
        // ------------------------------------------------------------
        // The transform pass populates componentIds for every component .ts.
        // A change here is either:
        //   (a) only the inline `template:` and/or `styles:` fields changed
        //       → HMR (no reload), matching Angular CLI's behavior.
        //   (b) anything else (class body, imports, other decorator metadata)
        //       → full reload, since Angular's runtime can't safely hot-swap
        //       class definitions.
        const isTsFile = ANGULAR_TS_REGEX.test(ctx.file)
        if (isTsFile && componentIds.has(ctx.file)) {
          // If a pending update was already registered for this component
          // (e.g. an external template change just invalidated the .ts module
          // via the graph), the resource branch has it covered.
          if (pendingHmrUpdates.has(ctx.file)) {
            debugHmr('component .ts: pending HMR already queued, skip')
            return []
          }

          // Strip-based check: if the source with `template:` and `styles:`
          // decorator fields stripped is byte-identical to the cached stripped
          // form, the diff is contained entirely in those fields and we can
          // HMR. This covers inline-template-only, inline-style-only, and
          // both-at-once changes uniformly.
          const cachedStripped = componentMetadataCache.get(ctx.file)
          if (cachedStripped !== undefined) {
            let newContent: string
            try {
              newContent = readFileSync(ctx.file, 'utf-8')
            } catch {
              newContent = ''
            }
            const newStripped = stripComponentMetadata(newContent)
            if (newStripped === cachedStripped) {
              debugHmr('inline template/styles-only change, dispatching HMR for %s', ctx.file)
              const newTemplate = extractInlineTemplate(newContent)
              if (newTemplate !== null) inlineTemplateCache.set(ctx.file, newTemplate)
              const newStyles = extractInlineStyles(newContent)
              if (newStyles !== null) {
                inlineStylesCache.set(ctx.file, newStyles)
              } else {
                inlineStylesCache.delete(ctx.file)
              }
              componentMetadataCache.set(ctx.file, newStripped)
              dispatchComponentUpdate(ctx.file)
              return []
            }
          }

          // Anything else in a component .ts is a full reload.
          debugHmr('component .ts: triggering full reload for %s', ctx.file)
          const componentModule = ctx.server.moduleGraph.getModuleById(ctx.file)
          if (componentModule) ctx.server.moduleGraph.invalidateModule(componentModule)
          resourceCache.delete(normalizedFile)
          ctx.server.ws.send({ type: 'full-reload', path: ctx.file })
          return []
        }

        // ------------------------------------------------------------
        // Branch 3: plain (non-component) .ts
        // ------------------------------------------------------------
        // Utility modules, services, constants, route configs, type-only
        // files. Angular's runtime HMR only refreshes template/style
        // metadata on already-mounted instances; constants and bindings
        // captured by component constructors are not re-pulled. Vite's
        // default propagation accepts via the importing component's HMR
        // boundary without re-rendering — leaving the DOM stale. Match
        // Angular CLI's official behavior and full-reload.
        //
        // Use `normalizedFile` for the node_modules check — on Windows
        // `ctx.file` may contain backslashes; `normalizePath` converts to
        // forward slashes so the substring match works cross-platform.
        if (isTsFile && !normalizedFile.includes('/node_modules/')) {
          debugHmr('plain .ts: triggering full reload for %s', ctx.file)
          for (const mod of ctx.modules) {
            ctx.server.moduleGraph.invalidateModule(mod)
          }
          ctx.server.ws.send({ type: 'full-reload', path: ctx.file })
          return []
        }

        // ------------------------------------------------------------
        // Branch 4: anything else
        // ------------------------------------------------------------
        // Non-Angular files (json, images, etc.). Let Vite's default HMR
        // handle them.
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

/**
 * Extract inline styles array from @Component decorator.
 *
 * Handles `styles: [\`…\`]`, `styles: ['…']`, and `styles: ["…"]` and
 * combinations thereof. Returns null if no `styles:` array is present.
 */
function extractInlineStyles(code: string): string[] | null {
  const arrMatch = code.match(/styles\s*:\s*\[([\s\S]*?)\]/)
  if (!arrMatch) return null
  const body = arrMatch[1]
  // Match each string literal in the array body. Order matters for HMR
  // delivery since styles are positional.
  const stringRe = /`([\s\S]*?)`|'((?:\\.|[^'\\])*)'|"((?:\\.|[^"\\])*)"/g
  const styles: string[] = []
  let m: RegExpExecArray | null
  while ((m = stringRe.exec(body)) !== null) {
    styles.push(m[1] ?? m[2] ?? m[3] ?? '')
  }
  return styles.length > 0 ? styles : null
}

/**
 * Replace the inline `template:` and `styles:` decorator fields with empty
 * placeholders. Used to detect "only template and/or styles changed" — if
 * the stripped form of the old and new source is byte-identical, the diff
 * is contained within those fields and we can dispatch an HMR component
 * update instead of a full reload.
 *
 * Note: each replace targets the FIRST occurrence only. This assumes one
 * `@Component` decorator per file (Angular convention). Files with multiple
 * components fall through to full-reload, which is the safe default.
 */
function stripComponentMetadata(code: string): string {
  return code
    .replace(/template\s*:\s*`[\s\S]*?`/, 'template:``')
    .replace(/template\s*:\s*'(?:\\.|[^'\\])*'/, "template:''")
    .replace(/template\s*:\s*"(?:\\.|[^"\\])*"/, 'template:""')
    .replace(/styles\s*:\s*\[[\s\S]*?\]/, 'styles:[]')
}

export { angular as default }

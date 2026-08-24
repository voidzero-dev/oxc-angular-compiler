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
import type { Plugin, ResolvedConfig, ViteDevServer, Connect, ModuleNode } from 'vite'
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
import {
  emptyDelimitedRange,
  locateComponentDecorators,
  locateStylesFieldFor,
  locateStylesInArgs,
  locateTemplateInArgs,
  locateTemplateStringFor,
  locateTemplateUrlFor,
} from './utils/decorator-fields.js'
import { injectDtsDeclarations } from './utils/dts.js'

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

  /**
   * Emit `ɵsetClassMetadata()` calls for TestBed support.
   *
   * Mirrors `ngc`'s behavior: when enabled, the original decorator metadata is
   * preserved on the compiled class wrapped in `(typeof ngDevMode === "undefined"
   * || ngDevMode) && …`, so production bundles tree-shake it away. Required for
   * TestBed APIs that recompile components with provider overrides. Resolved
   * `templateUrl`/`styleUrls` are inlined into the metadata as `template`/`styles`
   * to satisfy Angular's JIT `componentNeedsResolution` check.
   *
   * Default: `true` — matches `ngc`, which always emits class metadata.
   */
  emitClassMetadata?: boolean

  /**
   * Compilation mode.
   *
   * - `'full'` (default) emits fully-resolved Ivy definitions
   *   (`ɵɵdefineComponent`, `ɵɵdefineDirective`, …) — what application
   *   builds need.
   * - `'partial'` emits partial declarations (`ɵɵngDeclareComponent`,
   *   `ɵɵngDeclareDirective`, …) — what library builds publish. Consumer
   *   apps then run this package's linker (already integrated for
   *   `node_modules` code) to expand the declarations back into full
   *   Ivy form at their build time.
   *
   * Setting `'partial'` on an app build is almost certainly wrong: the
   * runtime needs full definitions and the linker only runs on
   * `node_modules`. Use it from a separate library build pipeline
   * (e.g. rolldown/tsdown producing an FESM).
   *
   * @example
   * ```ts
   * angular({ compilationMode: 'partial' }) // ng-packagr-style library build
   * ```
   */
  compilationMode?: 'full' | 'partial'
}

// Match all TypeScript files - we'll filter by @Component/@Directive decorator in the handler
const ANGULAR_TS_REGEX = /\.tsx?$/
const TEMPLATE_REGEX = /\.html?$/
const ANGULAR_COMPONENT_PREFIX = '@ng/component'

/**
 * True when `mod` is the module graph node for `normalizedFile` itself, and
 * not a postfixed variant of it.
 *
 * Vite sets `mod.file` to `cleanUrl(mod.id)`, which strips `?` and `#`, so a
 * template that application code also imports as `./tpl.html?raw` is filed
 * under the same `file` and appears in `ctx.modules` too. Matching on `id`
 * keeps the two apart: an exact match means same file, no postfix.
 */
function isModuleForFile(mod: ModuleNode, normalizedFile: string): boolean {
  return !!mod.id && normalizePath(mod.id) === normalizedFile
}

/**
 * Make `mod` its own HMR boundary, so an update stops there instead of
 * propagating to the modules that import it.
 *
 * On the deprecated mixed module node, `isSelfAccepting` is a prototype
 * getter with no setter. The flag therefore goes on the client-environment
 * node it delegates to, which is both writable and the node Vite reads when
 * it propagates the update. The node is mutated in place: a spread copy
 * would drop every prototype getter, including the `id` Vite matches on.
 */
function markModuleSelfAccepting(mod: ModuleNode): void {
  const clientModule = (mod as { _clientModule?: { isSelfAccepting?: boolean } })._clientModule
  if (clientModule) {
    clientModule.isSelfAccepting = true
  }
}

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
    emitClassMetadata: options.emitClassMetadata ?? true,
    compilationMode: options.compilationMode ?? 'full',
  }

  let resolvedConfig: ResolvedConfig
  let viteServer: ViteDevServer | undefined
  let watchMode = false
  let inlineBuild: InlineBuildMinifyOptions | undefined
  let outputMinify: unknown

  // For each component .ts file, the set of component class names declared
  // inside it. A file can legally have multiple @Component classes — Angular
  // emits per-component HMR updates and we mirror that here.
  const componentsByFile = new Map<string, Set<string>>()

  // Reverse mapping: resource file path → component file path. Single-valued:
  // the last transform to reference a resource owns the slot. Multiple
  // components in the SAME file are covered by `dispatchAllComponentsInFile`;
  // a resource shared across component FILES is covered by the multi-valued
  // owner maps (`templateComponentOwners`, `styleComponentOwners`).
  const resourceToComponent = new Map<string, string>()

  // Cache for resolved resources
  const resourceCache = new Map<string, string>()

  // Preprocessor dependencies of each compiled style (Sass partials pulled in
  // through `@use`/`@import`/`meta.load-css`, Less imports, ...), plus the
  // reverse map from each dependency to the styles compiled from it, so that
  // editing a shared partial invalidates and re-dispatches every component
  // style built on top of it.
  const styleDepsCache = new Map<string, string[]>()
  const styleDepOwners = new Map<string, Set<string>>()

  // Every file used as a direct `styleUrl` of some component (normalized
  // paths). Tracked independently of `styleDepsCache`: a direct style that
  // fails preprocessing never gets a deps-cache entry, yet must still be
  // refreshed (and its now-valid imports registered) once the developer fixes
  // it. Keys are normalized so lookups from `handleHotUpdate` match on
  // Windows, where cache keys keep the platform-native separators.
  const directStyleUrls = new Set<string>()

  // Every file used as a direct `templateUrl` of some component (normalized
  // paths). A templateUrl is not required to end in `.html`, so template
  // ownership is tracked by this ROLE set, not by file extension.
  const directTemplateUrls = new Set<string>()

  // Direct component owners of each compiled style (normalized style path →
  // component files referencing it as a styleUrl). Unlike
  // `resourceToComponent`, this is multi-valued: a style shared by several
  // components must dispatch HMR to every one of them when the style or one of
  // its preprocessor deps changes.
  const styleComponentOwners = new Map<string, Set<string>>()

  // Direct component owners of each external template (normalized template
  // path → component files referencing it as a templateUrl). Mirrors
  // `styleComponentOwners`: a template shared by several component files must
  // dispatch HMR to every one of them when it changes (issue #445).
  const templateComponentOwners = new Map<string, Set<string>>()

  // Record the preprocessor dependencies of a compiled style and rebuild the
  // reverse map (dep -> owning styles). Replaces any previous registration for
  // the style, so it is safe to call again whenever the style is (re)compiled
  // with fresh deps — the initial transform, or HMR after the style file
  // changed its `@use`/`@import` list. The style itself is never registered as
  // its own dependency.
  function registerStyleDeps(stylePath: string, deps: Iterable<string> | undefined): void {
    const normalizedStylePath = normalizePath(stylePath)
    const fresh = deps ? Array.from(deps, (dep) => normalizePath(dep)) : []

    // Drop this style from its previously registered deps' owner sets.
    for (const oldDep of styleDepsCache.get(normalizedStylePath) ?? []) {
      if (oldDep === normalizedStylePath) continue
      const owners = styleDepOwners.get(oldDep)
      if (owners) {
        owners.delete(normalizedStylePath)
        if (owners.size === 0) styleDepOwners.delete(oldDep)
      }
    }

    styleDepsCache.set(normalizedStylePath, fresh)

    // Register this style as an owner of each fresh dep. Cache keys and owner
    // values are normalized so lookups from handleHotUpdate (which receives
    // normalized ctx.file paths) match on Windows, where path.resolve keeps
    // backslashes.
    for (const dep of fresh) {
      if (dep === normalizedStylePath) continue
      let owners = styleDepOwners.get(dep)
      if (!owners) styleDepOwners.set(dep, (owners = new Set()))
      owners.add(normalizedStylePath)
    }
  }

  // Re-read and re-preprocess a style file so its dependency registration in
  // `styleDepsCache`/`styleDepOwners` reflects the current `@use`/`@import`
  // set. Without this, a partial added or switched via HMR would never be
  // registered, and edits to it would not dispatch component updates until a
  // full reload or another component transform. Best-effort: on unreadable or
  // transiently-empty files (truncate phase of an atomic write) the previous
  // registration is kept.
  async function refreshStyleDeps(stylePath: string): Promise<void> {
    if (!resolvedConfig) return
    let content: string
    try {
      content = await readFile(stylePath, 'utf-8')
    } catch {
      return
    }
    if (!content.trim()) return
    try {
      const processed = await preprocessCSS(content, stylePath, resolvedConfig as any)
      registerStyleDeps(stylePath, processed.deps)
      // A style edited via HMR can pick up new deps (possibly outside the
      // dev-server root); register them with the watcher so their edits reach
      // `handleHotUpdate`.
      if (watchMode && viteServer && processed.deps) {
        for (const dep of processed.deps) viteServer.watcher?.add?.(dep)
      }
    } catch (e) {
      console.warn(`Failed to preprocess style: ${stylePath}`, e)
    }
  }

  // Component IDs (`filePath@ClassName`) queued for HMR delivery. Populated by
  // `handleHotUpdate` when an external resource or inline template/style change
  // is detected, and consumed by the `@ng/component` HTTP endpoint, which reads
  // it to decide whether to serve the update module or an empty response.
  const pendingHmrUpdates = new Set<string>()

  // Per-component caches keyed by `filePath@ClassName`. A multi-component file
  // contributes one entry per component to each map.
  const inlineTemplateCache = new Map<string, string>()
  const inlineStylesCache = new Map<string, string[]>()

  // Cache the source of each component .ts file with its `template:` and
  // `styles:` decorator fields stripped. If the stripped form is byte-identical
  // before and after a save, we know only the template / styles changed and
  // can dispatch an HMR update instead of a full reload.
  const componentMetadataCache = new Map<string, string>()

  // Angular Ivy `.d.ts` static member declarations collected across the build,
  // keyed by module id. Populated during `transform` in `compilationMode:
  // 'partial'` (library) builds and consumed by `dtsPlugin`'s `generateBundle`
  // to augment the declaration files a separate dts generator emits.
  //
  // Keyed by module (not class name) so `vite build --watch` rebuilds can evict
  // a module's prior declarations before re-transforming it. Otherwise removing
  // a decorator — which makes the quick decorator check early-return, skipping
  // the transform entirely — would leave the old `ɵfac`/`ɵcmp` entries in place
  // and `generateBundle` would re-inject Ivy metadata into a now-plain class.
  const collectedDtsDeclarations = new Map<string, Array<{ className: string; members: string }>>()

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

      const normalizedTemplatePath = normalizePath(templatePath)
      // Register as a direct template regardless of read outcome, mirroring
      // `directStyleUrls`: ownership is by role, not extension.
      directTemplateUrls.add(normalizedTemplatePath)
      let content = resourceCache.get(normalizedTemplatePath)
      if (!content) {
        try {
          content = await readFile(templatePath, 'utf-8')
          if (options.templateTransform) {
            content = options.templateTransform(content, templatePath)
          }
          resourceCache.set(normalizedTemplatePath, content)
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
      const normalizedStylePath = normalizePath(stylePath)
      // Register as a direct style regardless of preprocessing outcome, so the
      // HMR refresh still runs for styles that initially failed to compile.
      directStyleUrls.add(normalizedStylePath)
      dependencies.push(stylePath)

      let content = resourceCache.get(normalizedStylePath)
      if (!content) {
        try {
          content = await readFile(stylePath, 'utf-8')
          // Preprocess styles (SCSS, Less, etc.)
          if (resolvedConfig) {
            try {
              const processed = await preprocessCSS(content, stylePath, resolvedConfig as any)
              content = processed.code
              registerStyleDeps(stylePath, processed.deps)
            } catch (e) {
              console.warn(`Failed to preprocess style: ${stylePath}`, e)
            }
          }
          resourceCache.set(normalizedStylePath, content)
        } catch {
          console.warn(`Failed to read style: ${stylePath}`)
          continue
        }
      }

      // Watch each transitive dep (it may resolve outside the dev-server
      // root, e.g. a shared monorepo package), but never register it in
      // `resourceToComponent`: that map is single-owner per resource, and a
      // transitive dep would clobber the direct styleUrl/templateUrl mapping
      // of another component.
      for (const dep of styleDepsCache.get(normalizedStylePath) ?? []) {
        if (dep === normalizedStylePath) continue
        if (watchMode && viteServer) viteServer.watcher?.add?.(dep)
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
            const className = decodedComponentId.slice(atIndex + 1)
            const resolvedId = resolve(process.cwd(), fileId)

            // Only return an HMR update module if `handleHotUpdate` queued
            // one for this component. On initial page load there are no
            // pending updates, so we return an empty response. This prevents
            // ɵɵreplaceMetadata from being called unnecessarily during
            // initial load, which would re-create views and cause errors
            // with @Required() decorators.
            if (!pendingHmrUpdates.has(decodedComponentId)) {
              res.setHeader('Content-Type', 'text/javascript')
              res.setHeader('Cache-Control', 'no-cache')
              res.end('')
              return
            }

            // If the requested className isn't (or is no longer) in the
            // file, the pending slot is stale and would otherwise stick
            // around indefinitely because the transient-empty preservation
            // logic below assumes a future save will resolve it. Consume
            // and return empty.
            if (!componentsByFile.get(resolvedId)?.has(className)) {
              pendingHmrUpdates.delete(decodedComponentId)
              res.setHeader('Content-Type', 'text/javascript')
              res.setHeader('Cache-Control', 'no-cache')
              res.end('')
              return
            }

            try {
              const source = await readFile(resolvedId, 'utf-8')
              const { templateUrls, styleUrls } = await extractComponentUrls(source, resolvedId)
              const dir = dirname(resolvedId)

              // Read fresh template content (bypass cache for HMR). Resolve
              // it per CLASS: in a multi-component file, templateUrls[0] is
              // the FILE's first external template, which may belong to a
              // sibling — serving it would replace this class's template with
              // the sibling's markup. Prefer the requested class's own
              // templateUrl, then its inline template; fall back to
              // templateUrls[0] only for decorator shapes the per-class
              // locator cannot parse (preserves the old behavior there).
              const readTemplate = async (url: string) => {
                const templatePath = resolve(dir, url)
                let content = await readFile(templatePath, 'utf-8')
                if (options.templateTransform) {
                  content = options.templateTransform(content, templatePath)
                }
                return content
              }
              let templateContent: string | null = null
              const classTemplateUrl = extractTemplateUrlFor(source, className)
              if (classTemplateUrl !== null) {
                templateContent = await readTemplate(classTemplateUrl)
              } else {
                templateContent = extractInlineTemplate(source, className)
                if (templateContent === null && templateUrls.length > 0) {
                  templateContent = await readTemplate(templateUrls[0])
                }
              }

              if (templateContent) {
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
                  const inlineStyles = extractInlineStyles(source, className)
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
                pendingHmrUpdates.delete(decodedComponentId)
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
              pendingHmrUpdates.delete(decodedComponentId)

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

          // Library builds: evict any declarations this module contributed on a
          // previous (watch) pass before re-deriving them below. Done ahead of
          // the decorator early-return so a class that just lost its decorator
          // doesn't keep stale Ivy metadata in the regenerated `.d.ts`.
          if (pluginOptions.compilationMode === 'partial') {
            collectedDtsDeclarations.delete(id)
          }

          // Quick check for Angular decorators - avoids parsing files without them
          // OXC handles @Component, @Directive, @NgModule, @Injectable, @Pipe, @Service
          const hasAngularDecorator =
            code.includes('@Component') ||
            code.includes('@Directive') ||
            code.includes('@NgModule') ||
            code.includes('@Injectable') ||
            code.includes('@Pipe') ||
            code.includes('@Service')
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
          // `handleHotUpdate` below dispatches based on `resourceToComponent`
          // membership. Preprocessor deps can resolve outside the root
          // (shared monorepo packages, configured include paths), so they are
          // registered with the watcher explicitly below.
          //
          // `addWatchFile` does more than watch: `vite:import-analysis` folds
          // plugin-added imports into the module graph, so each resource
          // becomes a real module with this component `.ts` as its importer.
          // That is load-bearing for templates. Vite full-reloads a changed
          // `.html` whose module list is empty or holds no `js` module, and a
          // template we hot-swap ourselves must not be in either state. See
          // `handleHotUpdate` and https://github.com/voidzero-dev/oxc-angular-compiler/issues/443.
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
            for (const [style, owners] of styleComponentOwners) {
              if (owners.has(actualId) && !newDeps.has(style)) {
                owners.delete(actualId)
                if (owners.size === 0) styleComponentOwners.delete(style)
              }
            }
            for (const [template, owners] of templateComponentOwners) {
              if (owners.has(actualId) && !newDeps.has(template)) {
                owners.delete(actualId)
                if (owners.size === 0) templateComponentOwners.delete(template)
              }
            }

            for (const dep of dependencies) {
              const normalizedDep = normalizePath(dep)
              // Track reverse mapping for HMR: resource → component
              resourceToComponent.set(normalizedDep, actualId)
              // Every component that uses a style directly is an owner of it.
              if (directStyleUrls.has(normalizedDep)) {
                let owners = styleComponentOwners.get(normalizedDep)
                if (!owners) styleComponentOwners.set(normalizedDep, (owners = new Set()))
                owners.add(actualId)
              }
              // Every component that uses an external template is an owner of
              // it. Membership is by ROLE (`directTemplateUrls`), not by
              // extension — a templateUrl may point at any file name.
              if (directTemplateUrls.has(normalizedDep)) {
                let owners = templateComponentOwners.get(normalizedDep)
                if (!owners) templateComponentOwners.set(normalizedDep, (owners = new Set()))
                owners.add(actualId)
              }
              // Watch the file so edits reach `handleHotUpdate` even when it
              // lives outside the dev-server root.
              viteServer.watcher?.add?.(dep)
              // Templates only, and only while HMR is on.
              //
              // Styles must stay out of the graph: the import edge makes Vite
              // propagate a style change up to this component `.ts` and
              // re-execute it, which defines a duplicate class and leaves
              // `angular:component-update` patching a class that is no longer
              // mounted. Styles never needed it — Vite only force-reloads on
              // `.html`.
              //
              // With `liveReload: false`, `handleHotUpdate` returns before it
              // can use the edge, so the edge has no consumer other than
              // Vite's default propagation — which is exactly what turns a
              // payload the client would have dropped into an unconditional
              // `full-reload` with path `*`.
              if (pluginOptions.liveReload && TEMPLATE_REGEX.test(normalizedDep)) {
                this.addWatchFile(dep)
              }
            }
          }

          // Transform with Rust compiler
          const transformOptions: TransformOptions = {
            sourcemap: pluginOptions.sourceMap,
            jit: pluginOptions.jit,
            hmr: pluginOptions.liveReload && watchMode && !isSSR,
            angularVersion: pluginOptions.angularVersion,
            minifyComponentStyles: getMinifyComponentStyles(this as any),
            emitClassMetadata: pluginOptions.emitClassMetadata,
            compilationMode: pluginOptions.compilationMode,
          }

          const result = await transformAngularFile(code, actualId, transformOptions, resources)

          // Report errors and warnings
          for (const error of result.errors) {
            this.error(error.message)
          }
          for (const warning of result.warnings) {
            this.warn(warning.message)
          }

          // Library builds: stash the Ivy `.d.ts` member declarations for this
          // file so `dtsPlugin` can splice them into the emitted declarations.
          if (pluginOptions.compilationMode === 'partial' && result.dtsDeclarations.length > 0) {
            collectedDtsDeclarations.set(
              id,
              result.dtsDeclarations.map((decl) => ({
                className: decl.className,
                members: decl.members,
              })),
            )
          }

          // Track component IDs for HMR — one entry per @Component class.
          if (pluginOptions.liveReload) {
            // templateUpdates is keyed by `filePath@ClassName` (NAPI HashMap → JS object).
            const templateUpdateKeys = Object.keys(result.templateUpdates)
            debugTransform(
              'transform %s templateUpdates=%O deps=%O',
              actualId,
              templateUpdateKeys,
              dependencies,
            )
            const classNamesInFile = new Set<string>()
            for (const componentId of templateUpdateKeys) {
              const atIdx = componentId.indexOf('@')
              if (atIdx === -1) continue
              classNamesInFile.add(componentId.slice(atIdx + 1))
            }
            // Prune cache entries for components that USED to be in this file
            // but no longer are (e.g. a class was renamed or removed). Without
            // this, the HMR endpoint could find a stale `pendingHmrUpdates`
            // entry pointing at a className that's gone, fail to extract a
            // template for it, and orphan the slot forever.
            const previouslyInFile = componentsByFile.get(actualId)
            if (previouslyInFile) {
              for (const oldClass of previouslyInFile) {
                if (classNamesInFile.has(oldClass)) continue
                const staleKey = `${actualId}@${oldClass}`
                inlineTemplateCache.delete(staleKey)
                inlineStylesCache.delete(staleKey)
                pendingHmrUpdates.delete(staleKey)
                debugHmr('pruned stale cache entries for %s', staleKey)
              }
            }

            componentsByFile.set(actualId, classNamesInFile)
            for (const className of classNamesInFile) {
              debugHmr('registered: %s -> %s', actualId, className)
            }

            // Cache per-component inline template / styles for detecting
            // template/styles-only changes in handleHotUpdate, and the
            // metadata-stripped (whole-file) source for cheaply diffing
            // whether anything else changed.
            for (const className of classNamesInFile) {
              const cacheKey = `${actualId}@${className}`
              const inlineTemplate = extractInlineTemplate(code, className)
              if (inlineTemplate !== null) {
                inlineTemplateCache.set(cacheKey, inlineTemplate)
              } else {
                inlineTemplateCache.delete(cacheKey)
              }
              const inlineStyles = extractInlineStyles(code, className)
              if (inlineStyles !== null) {
                inlineStylesCache.set(cacheKey, inlineStyles)
              } else {
                inlineStylesCache.delete(cacheKey)
              }
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

        // Helper: dispatch an HMR update for a specific component (identified
        // by its componentFile + className). Used by both external resource
        // and inline template/style branches. Returns true if dispatched.
        const dispatchComponentUpdate = (componentFile: string, className: string): boolean => {
          const classNames = componentsByFile.get(componentFile)
          if (!classNames || !classNames.has(className)) return false

          const componentId = `${componentFile}@${className}`
          // The HMR HTTP endpoint reads this set to decide whether to serve
          // the update module or an empty response.
          pendingHmrUpdates.add(componentId)

          // Invalidate the component's module so the next request reads fresh
          // template/style content. Module is per-file; safe to invalidate
          // once even if multiple components share it (subsequent dispatch
          // calls for siblings will no-op the invalidation).
          const mod = ctx.server.moduleGraph.getModuleById(componentFile)
          if (mod) ctx.server.moduleGraph.invalidateModule(mod)

          const encodedId = encodeURIComponent(componentId)
          debugHmr('sending angular:component-update id=%s', encodedId)
          ctx.server.ws.send({
            type: 'custom',
            event: 'angular:component-update',
            data: { id: encodedId, timestamp: Date.now() },
          })
          return true
        }

        // Dispatch an HMR event for every component in the given file. Used
        // when a whole-file diff (or external resource) tells us the change
        // is contained within template/styles but we can't cheaply attribute
        // it to a specific component. Angular's runtime no-ops
        // ɵɵreplaceMetadata when the metadata didn't actually change, so
        // over-dispatching is safe.
        const dispatchAllComponentsInFile = (componentFile: string): boolean => {
          const classNames = componentsByFile.get(componentFile)
          if (!classNames || classNames.size === 0) return false
          let dispatched = false
          for (const className of classNames) {
            if (dispatchComponentUpdate(componentFile, className)) dispatched = true
          }
          return dispatched
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
        // Every Vite-supported stylesheet language (CSS, Sass/SCSS, Less,
        // Stylus, PostCSS, SugarSS) plus HTML templates: preprocessCSS reports
        // deps for all of them, and their partials must reach this branch.
        if (/\.(html?|css|scss|sass|less|styl|stylus|pcss|postcss|sss)$/.test(ctx.file)) {
          let handled = false
          // Owner files already sent an update during this invocation. The
          // `ws.send` in `dispatchComponentUpdate` is not idempotent, so an
          // owner reached more than once below would otherwise emit a second
          // event for every class in it (issue #453).
          const dispatchedOwners = new Set<string>()
          // Dispatch every component in one owner file, at most once per
          // invocation. An owner can be reached several times over: by two of
          // its own styles sharing a partial, by a partial that is also one of
          // its direct styleUrls, or by a file it uses in both decorator
          // roles. Every repeat is redundant — the update is keyed by
          // `file@class` alone, and the HMR endpoint rereads the component's
          // resources from disk, so the first event already carries every
          // change to any of them.
          const dispatchOwner = (owner: string): boolean => {
            if (dispatchedOwners.has(owner)) return false
            dispatchedOwners.add(owner)
            if (!dispatchAllComponentsInFile(owner)) return false
            handled = true
            return true
          }
          // Shared preprocessor dependency (e.g. a Sass partial): rebuild every
          // style compiled from it and HMR each owning component.
          if (styleDepOwners.has(normalizedFile)) {
            // Snapshot the owners: refreshStyleDeps mutates the owner sets via
            // registerStyleDeps, and a re-added style would otherwise be
            // visited again during live Set iteration.
            for (const stylePath of Array.from(styleDepOwners.get(normalizedFile)!)) {
              resourceCache.delete(stylePath)
              // Rebuild the owning style's dependency registration: its dep
              // list includes the edited partial transitively, and if that
              // partial switched a nested `@use`/`@import`, the newly loaded
              // file must be tracked here too.
              await refreshStyleDeps(stylePath)
              // A style shared by several components updates every one of
              // them (resourceToComponent is single-valued).
              for (const owner of styleComponentOwners.get(normalizePath(stylePath)) ?? []) {
                if (dispatchOwner(owner)) {
                  debugHmr('style dep HMR: %s -> %s -> %s', normalizedFile, stylePath, owner)
                }
              }
            }
          }
          // A changed file can be BOTH a shared dep of one component's style
          // and another component's direct templateUrl/styleUrl — process both
          // roles before returning (no early return above). Direct styles and
          // templates are also reachable via their owner maps alone: once the
          // last resourceToComponent owner switches resources, its prune
          // removes the single-valued entry while the remaining shared owners
          // stay.
          if (
            resourceToComponent.has(normalizedFile) ||
            styleComponentOwners.has(normalizedFile) ||
            templateComponentOwners.has(normalizedFile)
          ) {
            // The two decorator roles are independent, not alternatives: one
            // file can be a styleUrl of one component and a templateUrl of
            // another, so both owner sets are dispatched (issue #450).
            const isDirectStyle = directStyleUrls.has(normalizedFile)
            const isDirectTemplate = directTemplateUrls.has(normalizedFile)
            // Stylesheets that only appear as transitive deps of other styles
            // (never used as a direct styleUrl or templateUrl) were already
            // handled by the shared-dep branch; skip them here to avoid a
            // duplicate update.
            if (!(handled && !isDirectStyle && !isDirectTemplate)) {
              resourceCache.delete(normalizedFile)
              if (isDirectStyle) {
                // Refresh dependency registration only for actual styles —
                // never run HTML templates through the CSS preprocessor
                // pipeline.
                await refreshStyleDeps(ctx.file)
                // A style shared by several components updates every one of
                // them (resourceToComponent is single-valued).
                for (const owner of styleComponentOwners.get(normalizedFile) ?? []) {
                  if (dispatchOwner(owner)) {
                    debugHmr('external resource HMR: %s -> %s', normalizedFile, owner)
                  }
                }
              }
              if (isDirectTemplate) {
                // A template shared by several component files updates every
                // one of them (resourceToComponent is single-valued). An owner
                // already updated above is skipped by `dispatchOwner`.
                for (const owner of templateComponentOwners.get(normalizedFile) ?? []) {
                  if (dispatchOwner(owner)) {
                    debugHmr('external resource HMR: %s -> %s', normalizedFile, owner)
                  }
                }
              }
            }
          }
          // Angular HMR (component updates) has been dispatched for any
          // tracked resources. Modules still in Vite's graph — e.g. a global
          // stylesheet that imports the same partial — must keep flowing
          // through Vite's default pipeline; returning [] would drop them and
          // leave that CSS stale.
          //
          // A template we just hot-swapped is the one exception. Vite sends a
          // `full-reload` for any changed `.html` whose module list is empty
          // or holds no `js` module. Its client normally drops that payload
          // because the path does not match `location.pathname` — but in
          // `middlewareMode` the path is `*`, which always reloads. So the
          // update is applied and the page reloads on top of it (issue #443).
          //
          // `addWatchFile` in `transform` already put the template in the
          // graph as a `js` module, which clears that branch. Marking it
          // self-accepting makes it its own HMR boundary, so the update stops
          // there instead of propagating up and re-executing the component
          // `.ts` — a second evaluation would define a duplicate class
          // (NG0912) and leave `angular:component-update` patching a class
          // that is no longer mounted.
          //
          // The browser never imported the template, so Vite's client finds
          // no `hotModulesMap` entry and the update is a no-op there. The DOM
          // change comes entirely from `angular:component-update` above.
          //
          // Only the template's own node is marked. A variant the browser did
          // import — `./tpl.html?raw` and friends — keeps propagating to its
          // importers, which would otherwise hold a stale value: Vite would
          // stop at a module with no `import.meta.hot.accept` handler.
          if (handled && TEMPLATE_REGEX.test(normalizedFile)) {
            for (const mod of ctx.modules) {
              if (isModuleForFile(mod, normalizedFile)) markModuleSelfAccepting(mod)
            }
          }
          return ctx.modules
        }

        // ------------------------------------------------------------
        // Branch 2: component .ts (has @Component decorator)
        // ------------------------------------------------------------
        // The transform pass populates componentsByFile for every component .ts.
        // A change here is either:
        //   (a) only the inline `template:` and/or `styles:` fields changed
        //       → HMR (no reload), matching Angular CLI's behavior.
        //   (b) anything else (class body, imports, other decorator metadata)
        //       → full reload, since Angular's runtime can't safely hot-swap
        //       class definitions.
        const isTsFile = ANGULAR_TS_REGEX.test(ctx.file)
        if (isTsFile && componentsByFile.has(ctx.file)) {
          // If a pending update is already queued for ANY component in this
          // file (e.g. an external template change just invalidated the .ts
          // module via the graph), the resource branch has it covered.
          const fileClassNames = componentsByFile.get(ctx.file)!
          let alreadyPending = false
          for (const className of fileClassNames) {
            if (pendingHmrUpdates.has(`${ctx.file}@${className}`)) {
              alreadyPending = true
              break
            }
          }
          if (alreadyPending) {
            debugHmr('component .ts: pending HMR already queued, skip')
            return []
          }

          // Strip-based check: if the source with EVERY @Component's
          // `template:` and `styles:` fields stripped is byte-identical to
          // the cached stripped form, the diff is contained entirely in
          // those fields (for one or more components in the file) and we
          // can HMR. This covers inline-template-only, inline-style-only,
          // and both-at-once changes uniformly, including the multi-component
          // case (a single edit to one component's template still satisfies
          // the equality because the other components' stripped fields are
          // identical before/after).
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
              // Refresh per-component caches with the new contents.
              for (const className of fileClassNames) {
                const cacheKey = `${ctx.file}@${className}`
                const newTemplate = extractInlineTemplate(newContent, className)
                if (newTemplate !== null) {
                  inlineTemplateCache.set(cacheKey, newTemplate)
                } else {
                  inlineTemplateCache.delete(cacheKey)
                }
                const newStyles = extractInlineStyles(newContent, className)
                if (newStyles !== null) {
                  inlineStylesCache.set(cacheKey, newStyles)
                } else {
                  inlineStylesCache.delete(cacheKey)
                }
              }
              componentMetadataCache.set(ctx.file, newStripped)
              // Conservatively dispatch HMR for every component in the file —
              // Angular's runtime no-ops if a component's metadata didn't
              // actually change. Per-component diffing is an easy follow-up.
              dispatchAllComponentsInFile(ctx.file)
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
  /**
   * Augment library `.d.ts` files with Angular's Ivy type declarations.
   *
   * Vite/Rolldown don't emit declarations themselves — a separate dts
   * generator (rolldown-plugin-dts, vite-plugin-dts, tsdown, `tsc`) produces
   * the base `.d.ts`. This plugin runs after them (`enforce: 'post'`) and
   * splices the static `ɵfac`/`ɵcmp`/… members collected during `transform`
   * into the matching classes so consumers get full template type-checking.
   *
   * Only active in `compilationMode: 'partial'` (library) builds; app builds
   * collect nothing, so this is a no-op there.
   */
  function dtsPlugin(): Plugin {
    return {
      name: '@oxc-angular/vite-dts',
      enforce: 'post',
      generateBundle(_outputOptions, bundle) {
        if (pluginOptions.compilationMode !== 'partial') return
        if (collectedDtsDeclarations.size === 0) return

        // Flatten every module's declarations into a class-name-keyed list.
        // A library publishes one class per name; if names ever collide the
        // last module wins, matching the previous (class-name-keyed) behavior.
        const byClassName = new Map<string, string>()
        for (const moduleDecls of collectedDtsDeclarations.values()) {
          for (const decl of moduleDecls) {
            byClassName.set(decl.className, decl.members)
          }
        }
        const declarations = Array.from(byClassName, ([className, members]) => ({
          className,
          members,
        }))

        for (const file of Object.values(bundle)) {
          if (file.type !== 'asset') continue
          if (!file.fileName.endsWith('.d.ts')) continue

          const source =
            typeof file.source === 'string'
              ? file.source
              : Buffer.from(file.source).toString('utf-8')

          const augmented = injectDtsDeclarations(source, declarations)
          if (augmented !== source) {
            file.source = augmented
          }
        }
      },
    }
  }

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
    dtsPlugin(),
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
 * Extract the inline template from the `@Component({...})` decorator that
 * decorates the class named `className`. Returns null if no such decorator
 * exists or the decorator has no inline `template:` string literal.
 */
function extractInlineTemplate(code: string, className: string): string | null {
  const range = locateTemplateStringFor(code, className)
  if (!range) return null
  // Slice excludes the outer quotes/backticks — raw inner contents.
  return code.slice(range[0] + 1, range[1])
}

/**
 * Extract the `templateUrl` string from the `@Component({...})` decorator
 * that decorates the class named `className`. Returns null if no such
 * decorator exists or the decorator has no `templateUrl:` string literal.
 */
function extractTemplateUrlFor(code: string, className: string): string | null {
  const range = locateTemplateUrlFor(code, className)
  if (!range) return null
  // Slice excludes the outer quotes/backticks — raw inner contents.
  return code.slice(range[0] + 1, range[1])
}

/**
 * Extract the inline styles from the `@Component({...})` decorator that
 * decorates the class named `className`, as a positional array.
 *
 * Handles both Angular forms — `styles: string | string[]`:
 *   - Array of literals (`['…']`, `["…"]`, `` [`…`] ``, or any mix) → each
 *     literal becomes one element, preserving order (HMR delivery is positional).
 *   - Bare single literal (`'…'`, `"…"`, or `` `…` ``) → returned as a
 *     one-element array.
 *
 * Returns null if the named decorator has no `styles:` field or its value is
 * something other than a string/array literal (e.g. a variable reference).
 */
function extractInlineStyles(code: string, className: string): string[] | null {
  const range = locateStylesFieldFor(code, className)
  if (!range) return null
  const opener = code[range[0]]
  if (opener !== '[') {
    // Bare string form — return the inner contents as a single element.
    return [code.slice(range[0] + 1, range[1])]
  }
  // Array form — walk string literals inside the array body in order.
  const body = code.slice(range[0] + 1, range[1])
  const stringRe = /`([\s\S]*?)`|'((?:\\.|[^'\\])*)'|"((?:\\.|[^"\\])*)"/g
  const styles: string[] = []
  let m: RegExpExecArray | null
  while ((m = stringRe.exec(body)) !== null) {
    styles.push(m[1] ?? m[2] ?? m[3] ?? '')
  }
  return styles.length > 0 ? styles : null
}

/**
 * Empty the `template:` and `styles:` field values of *every* `@Component(...)`
 * in the source, returning the result. Used to detect "only template/styles
 * changed somewhere in the file" — if the stripped form of the old and new
 * source is byte-identical, the diff is contained within those fields and we
 * can dispatch HMR (one event per component in the file) instead of a full
 * reload.
 */
function stripComponentMetadata(code: string): string {
  // Enumerate decorators ONCE (O(N) walk of source) and look up each one's
  // template + styles range directly from its argsRange. Calling the
  // className-based locators per decorator would re-enumerate inside each,
  // giving O(N²).
  //
  // Splice from highest start → lowest so earlier offsets stay valid as we
  // mutate the string from the end backwards.
  const decorators = locateComponentDecorators(code)
  const ranges: Array<[number, number]> = []
  for (const d of decorators) {
    const tpl = locateTemplateInArgs(code, d.argsRange)
    if (tpl) ranges.push(tpl)
    const styles = locateStylesInArgs(code, d.argsRange)
    if (styles) ranges.push(styles)
  }
  ranges.sort((a, b) => b[0] - a[0])
  return ranges.reduce((acc, range) => emptyDelimitedRange(acc, range), code)
}

export { angular as default }

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
import { dirname, isAbsolute, resolve } from 'node:path'

import { createDebug } from 'obug'
import type { Plugin, ResolvedConfig, ViteDevServer, Connect, ModuleNode } from 'vite'
import { preprocessCSS, normalizePath } from 'vite'

// Debug loggers - enable with DEBUG=vite:oxc-angular:*
const debugHmr = createDebug('vite:oxc-angular:hmr')
const debugTransform = createDebug('vite:oxc-angular:transform')

import {
  transformAngularFile,
  extractComponentUrls,
  compileComponentHmrUpdate,
  componentHmrFingerprint,
  type TransformOptions,
  type ResolvedResources,
  type AngularVersion,
} from '#binding'

import { buildOptimizerPlugin } from './angular-build-optimizer-plugin.js'
import { jitPlugin } from './angular-jit-plugin.js'
import { angularLinkerPlugin } from './angular-linker-plugin.js'
import { ssrManifestPlugin } from './angular-ssr-manifest-plugin.js'
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
   * Also used when linking partially compiled libraries (`ɵɵngDeclare*`).
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
 * `ResolvedResources` as the plugin builds it: NAPI-RS binds its `HashMap`s
 * to plain objects at runtime, despite the `Map` type in index.d.ts.
 */
interface PlainResources {
  templates: Record<string, string>
  styles: Record<string, string[]>
}

function plainResources(resources: ResolvedResources): PlainResources {
  return resources as unknown as PlainResources
}

/**
 * The text to report for a compiler diagnostic. Rollup shows only this string, so the
 * help text is appended to it: for a template parse error that is where the error is
 * (`file:line:column`).
 */
function describeDiagnostic(diagnostic: { message: string; helpMessage?: string | null }): string {
  return diagnostic.helpMessage
    ? `${diagnostic.message}\n${diagnostic.helpMessage}`
    : diagnostic.message
}

/**
 * True for the compiler's COMPONENT_RESOURCE_NOT_FOUND error: a `templateUrl`,
 * `styleUrl(s)` entry or template stylesheet `<link>` missing from the
 * resources. Diagnostics carry no code field, so the code is matched in the
 * message, where the compiler always writes it as `(COMPONENT_RESOURCE_NOT_FOUND)`.
 */
function isResourceNotFound(diagnostic: { message: string }): boolean {
  return diagnostic.message.includes('(COMPONENT_RESOURCE_NOT_FOUND)')
}

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

/** A component stylesheet after `preprocessCSS`, with the source it came from. */
type CompiledStyle = {
  source: string
  code: string
  deps: string[] | undefined
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
    // Like Angular CLI's `normalizeSourceMaps`, vendor maps are off unless the
    // object form turns them on; `sourceMap: true` only covers scripts.
    vendorSourceMap: typeof options.sourceMap === 'object' && (options.sourceMap.vendor ?? false),
    zoneless: options.zoneless ?? false,
    fileReplacements,
    angularVersion: options.angularVersion,
    emitClassMetadata: options.emitClassMetadata ?? true,
    compilationMode: options.compilationMode ?? 'full',
    tsconfig: options.tsconfig,
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

  // `preprocessCSS` output for each component stylesheet (normalized path),
  // tagged with the source text it was built from. Without it one stylesheet
  // edit is preprocessed up to three times: by `refreshStyleDeps` in
  // `handleHotUpdate`, by the `@ng/component` endpoint, and by the next
  // `transform` of the component.
  //
  // An entry only lives for one HMR cycle: `handleHotUpdate` clears the map
  // as it starts. The output can depend on files that are not in the
  // reported `deps` (Tailwind scans templates for class names), so a later
  // template edit must still rebuild the styles the endpoint serves, as it
  // always has.
  const compiledStyles = new Map<string, CompiledStyle>()
  // Bumped with every clear, so a compile that started in an earlier cycle
  // cannot store its (possibly stale) output in the new one.
  let styleCycle = 0

  function startStyleCycle(): void {
    styleCycle++
    compiledStyles.clear()
  }

  // Preprocess a component stylesheet (SCSS, Less, PostCSS, ...), reusing the
  // output built earlier in this HMR cycle from the same source text. Throws
  // what `preprocessCSS` throws; failures are not cached.
  async function compileStyle(stylePath: string, source: string): Promise<CompiledStyle> {
    const key = normalizePath(stylePath)
    const cached = compiledStyles.get(key)
    if (cached && cached.source === source) return cached
    const cycle = styleCycle
    const processed = await preprocessCSS(source, stylePath, resolvedConfig as any)
    const compiled: CompiledStyle = {
      source,
      code: processed.code,
      deps: processed.deps ? Array.from(processed.deps) : undefined,
    }
    // Builds never start a cycle and `resourceCache` already serves their
    // repeat reads, so only the dev server keeps entries.
    if (watchMode && cycle === styleCycle) compiledStyles.set(key, compiled)
    return compiled
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
      const processed = await compileStyle(stylePath, content)
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

  // Register template stylesheet `<link>`s found by an HMR template compile
  // as direct styles of `owner`, the way `transform` registers its
  // dependencies: owner maps for `handleHotUpdate`, the watcher, and the
  // stylesheet's own preprocessor deps. Links a later template edit removes
  // stay registered until the component `.ts` is transformed again (an edit
  // to one then sends a harmless extra update).
  async function registerLinkStyles(owner: string, dir: string, urls: string[]): Promise<void> {
    if (!watchMode || !viteServer) return
    for (const url of new Set(urls)) {
      const stylePath = resolve(dir, url)
      const normalizedStylePath = normalizePath(stylePath)
      directStyleUrls.add(normalizedStylePath)
      resourceToComponent.set(normalizedStylePath, owner)
      let owners = styleComponentOwners.get(normalizedStylePath)
      if (!owners) styleComponentOwners.set(normalizedStylePath, (owners = new Set()))
      owners.add(owner)
      viteServer.watcher?.add?.(stylePath)
      await refreshStyleDeps(stylePath)
    }
  }

  // Component IDs (`filePath@ClassName`) queued for HMR delivery. Populated by
  // `handleHotUpdate` when an external resource or inline template/style change
  // is detected, and consumed by the `@ng/component` HTTP endpoint, which reads
  // it to decide whether to serve the update module or an empty response.
  const pendingHmrUpdates = new Set<string>()

  // The HMR fingerprint of each component .ts file: a hash of its source with
  // the compiled components' inline `template:` / `styles:` values left out
  // (`TransformResult.hmrFingerprint`). If the fingerprint is the same before
  // and after a save, only the template / styles changed and we can dispatch
  // an HMR update instead of a full reload.
  const componentFingerprintCache = new Map<string, string>()

  // The resource URLs (as written) each component (`filePath@ClassName`)
  // needs: its `templateUrl`, then its `styleUrls` and template stylesheet
  // links. `transform` seeds them with the URLs it read for the file (all of
  // its components' URLs, as it cannot tell them apart), and each update
  // narrows them to the ones that component read. The `@ng/component`
  // endpoint reads them before it compiles, so an update needs one compile
  // instead of a first one that only learns which files to read.
  const hmrResourceUrls = new Map<string, { templateUrls: string[]; styleUrls: string[] }>()

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
  const collectedDtsDeclarations = new Map<
    string,
    Array<{
      className: string
      members: string
      namespaceImports: Record<string, string>
    }>
  >()

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
   * The options `transform` compiles `actualId` with. The `@ng/component`
   * endpoint compiles updates with the same ones.
   */
  function transformOptionsFor(
    actualId: string,
    isSSR: boolean,
    context?: Parameters<typeof getMinifyComponentStyles>[0],
  ): TransformOptions {
    return {
      sourcemap: pluginOptions.sourceMap,
      jit: pluginOptions.jit,
      // Disable HMR for SSR transforms. SSR bundles must not contain HMR
      // initialization code that dynamically imports @ng/component virtual
      // modules, as those are served via HTTP middleware only. This matches
      // Angular's official behavior where _enableHmr is only set for browser
      // bundles (see @angular/build application-code-bundle.js).
      hmr: pluginOptions.liveReload && watchMode && !isSSR,
      angularVersion: pluginOptions.angularVersion,
      minifyComponentStyles: getMinifyComponentStyles(context),
      emitClassMetadata: pluginOptions.emitClassMetadata,
      compilationMode: pluginOptions.compilationMode,
      // Evaluate decorator metadata imported from other files the way
      // ngtsc's program-wide checker does (`inputs: INPUTS`, `@Input(OPTS)`,
      // `templateUrl: TEMPLATE_URL`). Read files land in
      // `result.dependencies`, tracked by `transform`.
      resolveImportedValues: true,
      baseDir: dirname(actualId),
      tsconfigPath: pluginOptions.tsconfig,
    }
  }

  /**
   * Resolve external template/style URLs and read their contents.
   *
   * `transformOptions` are the ones the file is transformed with, so the URLs
   * are read as the transform will look them up (a `templateUrl` held in a
   * `const` imported from another file, say).
   */
  async function resolveResources(
    code: string,
    id: string,
    transformOptions: TransformOptions,
  ): Promise<{ resources: ResolvedResources; dependencies: string[] }> {
    const { templateUrls, styleUrls } = await extractComponentUrls(code, id, transformOptions)
    const dir = dirname(id)
    const { resources, dependencies } = emptyResources()
    const { templates, styles } = plainResources(resources)

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
      await resolveStyle(dir, styleUrl, styles, dependencies)
    }

    return { resources, dependencies }
  }

  /**
   * Read and preprocess one component stylesheet into `styles[styleUrl]`,
   * resolving `styleUrl` against `dir` (the component file's directory).
   * Serves decorator `styleUrl(s)` and template stylesheet `<link>`s alike:
   * both are direct styles of the component, for the cache, the watcher and
   * HMR. Returns whether the stylesheet could be read.
   */
  async function resolveStyle(
    dir: string,
    styleUrl: string,
    styles: Record<string, string[]>,
    dependencies: string[],
  ): Promise<boolean> {
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
            const processed = await compileStyle(stylePath, content)
            content = processed.code
            registerStyleDeps(stylePath, processed.deps)
          } catch (e) {
            console.warn(`Failed to preprocess style: ${stylePath}`, e)
          }
        }
        resourceCache.set(normalizedStylePath, content)
      } catch {
        console.warn(`Failed to read style: ${stylePath}`)
        return false
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
    return true
  }

  /**
   * What `resolveResources` returns for a file with no external template or
   * stylesheet.
   */
  function emptyResources(): { resources: ResolvedResources; dependencies: string[] } {
    const plain: PlainResources = { templates: {}, styles: {} }
    return { resources: plain as unknown as ResolvedResources, dependencies: [] }
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
            // `get` already decodes the percent-encoded id the HMR
            // initializer put in `id`; a second decodeURIComponent would
            // corrupt ids containing a literal `%` (e.g. `a%2Fb.ts`, or a
            // bare `%` which throws URIError).
            const componentId = requestUrl.searchParams.get('c')

            if (!componentId) {
              res.statusCode = 400
              res.end()
              return
            }

            const atIndex = componentId.lastIndexOf('@')

            // Validate component ID format: should be "filePath@ClassName"
            if (atIndex === -1) {
              console.error(`[Angular HMR] Invalid component ID format: ${componentId}`)
              res.statusCode = 400
              res.end()
              return
            }

            const fileId = componentId.slice(0, atIndex)
            const className = componentId.slice(atIndex + 1)
            // `fileId` is the transform id verbatim, which is what the per-file
            // maps are keyed by. `resolvedId` is for the filesystem only: on
            // Windows it swaps Vite's forward slashes for backslashes.
            const resolvedId = resolve(process.cwd(), fileId)

            // Only return an HMR update module if `handleHotUpdate` queued
            // one for this component. On initial page load there are no
            // pending updates, so we return an empty response. This prevents
            // ɵɵreplaceMetadata from being called unnecessarily during
            // initial load, which would re-create views and cause errors
            // with @Required() decorators.
            if (!pendingHmrUpdates.has(componentId)) {
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
            if (!componentsByFile.get(fileId)?.has(className)) {
              pendingHmrUpdates.delete(componentId)
              res.setHeader('Content-Type', 'text/javascript')
              res.setHeader('Cache-Control', 'no-cache')
              res.end('')
              return
            }

            try {
              const source = await readFile(resolvedId, 'utf-8')
              const dir = dirname(resolvedId)

              // One Rust call parses the file and compiles the class the way
              // `transform` does: same options (so `preserveWhitespaces`, i18n
              // message names, the Angular version and the file path behind
              // error locations match), same metadata evaluation (imported
              // values included), same style finishing. A class the transform
              // did not compile, or whose template is empty (the truncate
              // phase of an atomic write), gets no module.
              //
              // The compiler cannot read files, so it names the resources it
              // was not given: the class's `templateUrl` and `styleUrls`, then
              // the template's stylesheet `<link>`s, which are only known once
              // the template is parsed. They are read fresh from disk
              // (templates through `templateTransform`, stylesheets through
              // the cycle-shared `compileStyle`) and the class compiled again.
              // The URLs the last update needed are read up front, so a
              // repeat edit compiles once.
              //
              // A stylesheet that cannot be read stays missing. That is not
              // proof the component has no styles, so the compiler will not
              // turn it into a definitive `styles: []` that wipes live CSS;
              // nor will it for a disk source that is not the one `transform`
              // compiled (another plugin's `load` / `transform`, or
              // `fileReplacements`): the fingerprints differ then.
              const resources: PlainResources = { templates: {}, styles: {} }
              const attempted = new Set<string>()
              const readTemplate = async (url: string) => {
                const templatePath = resolve(dir, url)
                let content = await readFile(templatePath, 'utf-8')
                if (options.templateTransform) {
                  content = options.templateTransform(content, templatePath)
                }
                resources.templates[url] = content
              }
              const readStyle = async (url: string) => {
                const stylePath = resolve(dir, url)
                try {
                  let content = await readFile(stylePath, 'utf-8')
                  if (resolvedConfig) {
                    content = (await compileStyle(stylePath, content)).code
                  }
                  resources.styles[url] = [content]
                } catch {
                  // Missing, unreadable, or failed to preprocess: unknown.
                }
              }
              const known = hmrResourceUrls.get(componentId)
              if (known) {
                for (const url of known.templateUrls) {
                  await readTemplate(url).catch(() => {})
                }
                for (const url of known.styleUrls) await readStyle(url)
              }

              const compiledClassNames = [...(componentsByFile.get(fileId) ?? [])]
              const compile = () =>
                compileComponentHmrUpdate(
                  source,
                  fileId,
                  className,
                  transformOptionsFor(fileId, false),
                  resources as unknown as ResolvedResources,
                  {
                    sourceFingerprint: componentFingerprintCache.get(fileId),
                    compiledClassNames,
                  },
                )
              let result = await compile()
              for (;;) {
                const templateUrl = result.missingTemplateUrl
                const styleUrls = result.missingStyleUrls.filter(
                  (url) => !attempted.has(`s:${url}`),
                )
                const readsTemplate = templateUrl != null && !attempted.has(`t:${templateUrl}`)
                if (!readsTemplate && styleUrls.length === 0) break
                // A template that cannot be read throws: the update fails
                // and the page reloads (the catch below).
                if (readsTemplate) {
                  attempted.add(`t:${templateUrl}`)
                  await readTemplate(templateUrl)
                }
                for (const url of styleUrls) {
                  attempted.add(`s:${url}`)
                  await readStyle(url)
                }
                result = await compile()
              }

              // A failed compile must not be served as an (empty) update
              // module — that would call `ɵɵreplaceMetadata` with no
              // callback and throw inside the listener. Fall back to the
              // `angular:invalidate` full-reload path in the catch below.
              const hmrErrors = result.errors.filter((e) => e.severity === 'Error')
              if (hmrErrors.length > 0) {
                // Keep the help text (`file:line:column` for template parse
                // errors) so the invalidation message says where it is.
                throw new Error(hmrErrors.map(describeDiagnostic).join('\n'))
              }

              if (result.hmrModule) {
                hmrResourceUrls.set(componentId, {
                  templateUrls: result.templateUrl != null ? [result.templateUrl] : [],
                  styleUrls: [...result.styleUrls, ...result.templateStyleUrls],
                })
                // A template edit can add a link the last full transform
                // never saw, and this endpoint is all that runs for it.
                // Register the links as this component's direct styles so
                // later edits to them hot-update it too.
                if (result.templateStyleUrls.length > 0) {
                  await registerLinkStyles(fileId, dir, result.templateStyleUrls)
                }

                // Only consume the pending slot once we have real content to
                // serve. If we deleted unconditionally and the file was
                // transiently empty (truncate phase of an atomic write on
                // Linux), the next inotify event's request would find no
                // pending entry and deliver no HMR.
                pendingHmrUpdates.delete(componentId)
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
              pendingHmrUpdates.delete(componentId)

              // Send angular:invalidate event to trigger graceful full reload
              // This matches Angular's HMR error fallback pattern. The client
              // compares `d.id` against the ENCODED id literal in its
              // initializer, so the id must go out encoded — sending it raw
              // makes the comparison never match and drops the reload.
              server.ws.send({
                type: 'custom',
                event: 'angular:invalidate',
                data: { id: encodeURIComponent(componentId), message: errorMessage, error: true },
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
          // OXC handles @Component, @Directive, @NgModule, @Injectable, @Pipe, @Service.
          // The compiler also takes one imported from `@angular/core` under another
          // name (`@Cmp(...)`, `@ng.Component(...)`); such a file still names
          // `@angular/core` and the decorator.
          const hasAngularDecorator =
            code.includes('@Component') ||
            code.includes('@Directive') ||
            code.includes('@NgModule') ||
            code.includes('@Injectable') ||
            code.includes('@Pipe') ||
            code.includes('@Service') ||
            (code.includes('@angular/core') &&
              /\b(?:Component|Directive|NgModule|Injectable|Pipe|Service)\b/.test(code))
          if (!hasAngularDecorator) {
            return
          }

          // Apply file replacements
          const actualId = pluginOptions.fileReplacements?.[id] ?? id

          // Resolve external resources. JIT output never reads them: the Rust
          // JIT transform turns `templateUrl`/`styleUrl(s)` into
          // `angular:jit:*` imports that `jitPlugin` loads, and it registers
          // no components for HMR, so nothing downstream uses the reads or
          // the bookkeeping below.
          //
          // Outside JIT, `extractComponentUrls` is a second full parse of the
          // file. A file that never spells `templateUrl` or `styleUrl` almost
          // never has one, so it is compiled with no resources first; the
          // retry after the transform covers the rare file that does.
          const mayHaveResourceUrls =
            !pluginOptions.jit && (code.includes('templateUrl') || code.includes('styleUrl'))

          // Transform with Rust compiler. `extractComponentUrls` takes the same
          // options, so it evaluates the URLs exactly as the transform does.
          const transformOptions = transformOptionsFor(actualId, !!options?.ssr, this as any)

          let { resources, dependencies } = mayHaveResourceUrls
            ? await resolveResources(code, actualId, transformOptions)
            : emptyResources()

          let result = await transformAngularFile(code, actualId, transformOptions, resources)

          // The compiler reads a key the way the parser does, so a component
          // can still name a resource without the plain text: an escaped key
          // (`style\u0055rls`) or a computed key folded from a same-file
          // constant. It then reports the URL as COMPONENT_RESOURCE_NOT_FOUND,
          // since resources were passed but held none. Such an error on the
          // fast path reruns the file through the full resolution, which
          // makes the fast path give exactly the old result: with no URL in
          // the file, `resolveResources` would have passed these same empty
          // resources. Other errors (a template syntax error, say) would come
          // back the same from the rerun, so they do not trigger it. Missing
          // template `<link>`s (one error each) are left to the step below:
          // the URL extraction cannot find them.
          if (
            !pluginOptions.jit &&
            !mayHaveResourceUrls &&
            result.errors.filter(isResourceNotFound).length > result.missingLinkUrls.length
          ) {
            ;({ resources, dependencies } = await resolveResources(
              code,
              actualId,
              transformOptions,
            ))
            result = await transformAngularFile(code, actualId, transformOptions, resources)
          }

          // A template's `<link rel="stylesheet" href="...">` is one more
          // stylesheet of the component: ngtsc loads it like a `styleUrl`,
          // relative to the component file, and compiles it into `styles`.
          // Only the transform parses the template, so it reports the hrefs
          // it was not given; they are read and preprocessed like `styleUrls`
          // (and are direct styles for HMR), and the file is compiled once
          // more. Hrefs that still cannot be read stay
          // COMPONENT_RESOURCE_NOT_FOUND.
          if (!pluginOptions.jit && result.missingLinkUrls.length > 0) {
            const { styles } = plainResources(resources)
            let resolvedAny = false
            for (const linkUrl of new Set(result.missingLinkUrls)) {
              if (await resolveStyle(dirname(actualId), linkUrl, styles, dependencies)) {
                resolvedAny = true
              }
            }
            if (resolvedAny) {
              result = await transformAngularFile(code, actualId, transformOptions, resources)
            }
          }

          // Resource deps (templates/styles) plus files the compiler read to
          // resolve imported decorator-metadata values (`result.dependencies`).
          // Those `import` statements are often elided from the emitted JS —
          // the module graph would not cover `meta.ts`, so without this, edits
          // to metadata files leave stale compiled output. `result.dependencies`
          // also carries raw `templateUrl`/`styleUrl` strings — they can be
          // relative, so absolutize them against the component's directory.
          const componentDir = dirname(actualId)
          const allDeps = dependencies
            .concat(result.dependencies)
            .map((dep) => (isAbsolute(dep) ? dep : resolve(componentDir, dep)))

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
            const newDeps = new Set(allDeps.map(normalizePath))
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

            for (const dep of allDeps) {
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

          // Report errors and warnings
          for (const error of result.errors) {
            this.error(describeDiagnostic(error))
          }
          for (const warning of result.warnings) {
            this.warn(describeDiagnostic(warning))
          }

          // Library builds: stash the Ivy `.d.ts` member declarations for this
          // file so `dtsPlugin` can splice them into the emitted declarations.
          if (pluginOptions.compilationMode === 'partial' && result.dtsDeclarations.length > 0) {
            collectedDtsDeclarations.set(
              id,
              result.dtsDeclarations.map((decl) => ({
                className: decl.className,
                members: decl.members,
                namespaceImports: decl.namespaceImports,
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
              const atIdx = componentId.lastIndexOf('@')
              if (atIdx === -1) continue
              classNamesInFile.add(componentId.slice(atIdx + 1))
            }
            // Prune pending updates for components that USED to be in this
            // file but no longer are (e.g. a class was renamed or removed).
            // Without this, the HMR endpoint could find a stale
            // `pendingHmrUpdates` entry pointing at a className that's gone,
            // resolve no metadata for it, and orphan the slot forever.
            const previouslyInFile = componentsByFile.get(actualId)
            if (previouslyInFile) {
              for (const oldClass of previouslyInFile) {
                if (classNamesInFile.has(oldClass)) continue
                const staleKey = `${actualId}@${oldClass}`
                pendingHmrUpdates.delete(staleKey)
                hmrResourceUrls.delete(staleKey)
                debugHmr('pruned stale cache entries for %s', staleKey)
              }
            }

            componentsByFile.set(actualId, classNamesInFile)
            for (const className of classNamesInFile) {
              debugHmr('registered: %s -> %s', actualId, className)
            }

            // Hand the endpoint the resources this compile read: the
            // templates and stylesheets (template `<link>`s included) that
            // could be read, keyed by URL as written. Without them the first
            // update of each component would compile once to learn its
            // `templateUrl` and `styleUrls` and again for its `<link>`s.
            if (watchMode && !pluginOptions.jit) {
              const { templates, styles } = plainResources(resources)
              const urls = {
                templateUrls: Object.keys(templates),
                styleUrls: Object.keys(styles),
              }
              for (const className of classNamesInFile) {
                hmrResourceUrls.set(`${actualId}@${className}`, urls)
              }
            }

            // Cache the whole-file fingerprint for cheaply checking whether
            // anything besides template/styles changed. The transform takes
            // it from its own parse when `hmr` is on; otherwise (SSR, build)
            // no class was compiled for HMR and nothing is left out.
            componentFingerprintCache.set(
              actualId,
              result.hmrFingerprint ?? fingerprintOf(code, actualId, classNamesInFile),
            )
          }

          return {
            code: result.code,
            map: result.map ?? null,
          }
        },
      },
      async handleHotUpdate(ctx) {
        // Every file event starts a new HMR cycle for compiled styles, before
        // anything below can preprocess one (see `compiledStyles`).
        startStyleCycle()

        // The file's cached contents are stale, whatever its role now. A
        // template edited while its component fails to compile is not
        // registered as the component's at that moment (the failed transform
        // found no templateUrl), so the branches below would leave its old
        // contents cached for the component's next transform.
        resourceCache.delete(normalizePath(ctx.file))

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

          // Fingerprint check: if the source with EVERY compiled component's
          // `template:` and `styles:` values left out hashes the same as the
          // cached fingerprint, the diff is contained entirely in those
          // values (for one or more components in the file) and we can HMR.
          // This covers inline-template-only, inline-style-only, and
          // both-at-once changes uniformly, including the multi-component
          // case (a single edit to one component's template still keeps the
          // fingerprint, because everything outside the values is unchanged).
          const cachedFingerprint = componentFingerprintCache.get(ctx.file)
          if (cachedFingerprint !== undefined) {
            let newContent: string
            try {
              newContent = readFileSync(ctx.file, 'utf-8')
            } catch {
              newContent = ''
            }
            const newFingerprint = fingerprintOf(newContent, ctx.file, fileClassNames)
            if (newFingerprint === cachedFingerprint) {
              debugHmr('inline template/styles-only change, dispatching HMR for %s', ctx.file)
              componentFingerprintCache.set(ctx.file, newFingerprint)
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
        // `injectDtsDeclarations` emits relative specifiers verbatim — the
        // right behavior under structure-preserving declaration emit (the
        // dominant mode), where dist mirrors src. Bundled declarations that
        // collapse directories can't represent two same-named relative
        // modules regardless; that is a dts-generator limitation.
        if (pluginOptions.compilationMode !== 'partial') return
        if (collectedDtsDeclarations.size === 0) return

        // Flatten every module's declarations into a class-name-keyed list.
        // A library publishes one class per name; if names ever collide the
        // last module wins, matching the previous (class-name-keyed) behavior.
        // Keep each declaration's source module id: relative specifiers in
        // `namespaceImports` ("./dep") mean different modules under
        // different directories, so canonicalization keys on the resolved
        // identity, not the raw string.
        const byClassName = new Map<
          string,
          {
            className: string
            members: string
            namespaceImports: Record<string, string>
            sourceFile: string
          }
        >()
        for (const [moduleId, moduleDecls] of collectedDtsDeclarations) {
          for (const decl of moduleDecls) {
            byClassName.set(decl.className, { ...decl, sourceFile: moduleId })
          }
        }
        const declarations = Array.from(byClassName.values())

        for (const file of Object.values(bundle)) {
          if (file.type !== 'asset') continue
          if (!/\.d\.[cm]?ts$/.test(file.fileName)) continue

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

  return [
    angularPlugin(),
    dtsPlugin(),
    angularLinkerPlugin({
      angularVersion: pluginOptions.angularVersion,
      // The linker only sees node_modules files, so it follows the vendor setting
      // (Angular CLI only maps third-party code when both scripts and vendor are on).
      sourcemap: pluginOptions.sourceMap && pluginOptions.vendorSourceMap,
    }),
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
 * The HMR fingerprint of `code` for the components the transform compiled in
 * it (`classNames`): see `componentHmrFingerprint`. If the fingerprints of the
 * old and new source are equal, the diff is contained within those components'
 * inline `template:` / `styles:` values and we can dispatch HMR (one event per
 * component in the file) instead of a full reload.
 *
 * Only the class names travel between saves, not the decorators: the new
 * source is read through its own `@angular/core` imports, and a change that
 * could find a different decorator (an import) is outside the left-out values,
 * so it already makes the two fingerprints differ.
 */
function fingerprintOf(
  code: string,
  fileId: string,
  classNames: ReadonlySet<string> | undefined,
): string {
  return componentHmrFingerprint(code, fileId, classNames ? [...classNames] : []).fingerprint
}

export { angular as default }

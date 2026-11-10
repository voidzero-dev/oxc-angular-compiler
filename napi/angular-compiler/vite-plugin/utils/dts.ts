/**
 * Inject Angular's Ivy `.d.ts` type declarations into emitted declaration
 * files for library builds.
 *
 * The implementation lives in Rust (`dts_inject` in
 * `oxc_angular_compiler`, exported over napi as `injectDtsDeclarations`):
 * the compiler already carries oxc, so the emitted `.d.ts` is parsed with
 * the same engine rather than a JS-side parser dependency.
 *
 * The Rust compiler returns, per Angular class, the static member type
 * declarations that should live in the class's `.d.ts` body — e.g.
 * `static ɵcmp: i0.ɵɵComponentDeclaration<…>;`. Those members are what
 * Angular's template type-checker reads from a pre-compiled library, and
 * they mirror what ngtsc's `IvyDeclarationDtsTransform` would have written.
 *
 * Vite/Rolldown don't emit `.d.ts` themselves — a separate declaration
 * generator (rolldown-plugin-dts, vite-plugin-dts, tsdown, `tsc`) produces
 * the base declarations. This helper is a post-processing pass that splices
 * the Angular members into those already-generated `.d.ts`, and emits the
 * namespace imports the members reference.
 *
 * ngtsc never reuses an existing namespace import in declaration emit — its
 * `ImportManager` (`presetImportManagerForceNamespaceImports`) mints `i0`,
 * `i0_1`, … deduped against the ORIGINAL source file's identifiers. The
 * compiler mirrors that: each declaration's members carry the alias the
 * per-source-file registry picked, and `namespaceImports` records which
 * module each alias stands for. A bundled `.d.ts` merges declarations from
 * many source files whose aliases can disagree (`i1` → `./dep` in one,
 * `./other` in another), so the pass canonicalizes: one alias per module
 * identity, collision-free against every identifier in the emitted file.
 */
import { realpathSync, type Dirent } from 'node:fs'
import { readdir } from 'node:fs/promises'
import { dirname, isAbsolute, join, resolve } from 'node:path'

import {
  injectDtsDeclarations as injectDtsDeclarationsNative,
  injectDtsDeclarationsWithIdentityMap as injectDtsDeclarationsWithIdentityMapNative,
  injectDtsDeclarationsWithMap as injectDtsDeclarationsWithMapNative,
} from '#binding'

/** A single class's `.d.ts` static member declarations. */
export interface DtsClassDeclaration {
  /** The class the members belong to. */
  className: string
  /**
   * Newline-separated `static …;` member declarations. Namespace-qualified
   * type names (`i0.ɵɵComponentDeclaration`, `typeof i1.SomeDirective`) use
   * the alias the compiler picked for the member's module in that source
   * file; `namespaceImports` maps each such alias to its module specifier.
   */
  members: string
  /**
   * alias → module specifier for every namespace the members reference:
   * `i0` → `"@angular/core"` for `i0.ɵɵX`/`i0.Signal`, `i1` → `"./dep"`
   * for `typeof i1.SomeDirective` host-directive references, imported
   * ctor-dep types, and `ngAcceptInputType_*` transform types alike.
   * Relative specifiers (`"./dep"`) are resolved against `sourceFile` for
   * identity (so the same specifier under different directories is two
   * modules) and emitted verbatim — structure-preserving declaration emit
   * places `dist/a/foo.d.ts` next to `dist/a/dep.d.ts`, same as the source.
   */
  namespaceImports?: Record<string, string>
  /**
   * The source module the declaration was compiled from (the transform's
   * module id). Used to resolve relative specifiers in `namespaceImports`
   * so two files in different directories that each import `"./dep"`
   * canonicalize to different aliases instead of silently sharing one.
   */
  sourceFile?: string
}

/**
 * Splice each declaration's static members into the matching class body in
 * `source`, and emit the namespace imports the injected members reference —
 * one `import * as ns from "module"` per module identity, canonicalized
 * across all injected declarations.
 *
 * The pass is idempotent: a member whose STATIC name already appears in the
 * target class is skipped — structurally, on the member name, so a second
 * pass that canonicalizes to a different alias can't inject the same member
 * twice. A declaration whose class isn't found, whose members don't parse,
 * or a file the parser rejects is silently skipped.
 */
export function injectDtsDeclarations(
  source: string,
  declarations: readonly DtsClassDeclaration[],
): string {
  return injectDtsDeclarationsNative(
    source,
    declarations.map((d) => ({ ...d })),
  )
}

/** Injected code plus its source map JSON, when there is one. */
export interface DtsInjectWithMapResult {
  code: string
  map?: string
}

/**
 * {@link injectDtsDeclarations}, and also shift the `.d.ts`'s existing
 * declaration map (JSON) so it fits the new code, as ngtsc's
 * `afterDeclarations` emit would: original tokens keep their mapping,
 * injected text has none. Every other map field (`sources`, `sourceRoot`,
 * `x_google_ignoreList`, …) is kept. No `map` comes back when the input map
 * is missing or can't be read; the caller then leaves it as is.
 */
export function injectDtsDeclarationsWithMap(
  source: string,
  declarations: readonly DtsClassDeclaration[],
  map?: string | null,
): DtsInjectWithMapResult {
  const parsed = map ? parseMap(map) : undefined
  const result = injectDtsDeclarationsWithMapNative(
    source,
    declarations.map((d) => ({ ...d })),
    parsed?.mappings,
  )
  if (!parsed || result.mappings == null) {
    return { code: result.code }
  }
  return { code: result.code, map: JSON.stringify({ ...parsed, mappings: result.mappings }) }
}

/** A source map JSON object with a string `mappings`, or `undefined`. */
function parseMap(map: string): { mappings: string; [key: string]: unknown } | undefined {
  let value: unknown
  try {
    value = JSON.parse(map)
  } catch {
    return undefined
  }
  if (
    value &&
    typeof value === 'object' &&
    typeof (value as { mappings?: unknown }).mappings === 'string'
  ) {
    return value as { mappings: string }
  }
  return undefined
}

/**
 * {@link injectDtsDeclarations}, plus a source map JSON from the new code
 * back to `source` (one source, `sourceName`), e.g. for a Rollup/Rolldown
 * `transform` hook. Injected text is unmapped. No `map` comes back when
 * nothing was injected (the code is unchanged).
 */
export function injectDtsDeclarationsWithIdentityMap(
  source: string,
  declarations: readonly DtsClassDeclaration[],
  sourceName = '',
): DtsInjectWithMapResult {
  const result = injectDtsDeclarationsWithIdentityMapNative(
    source,
    declarations.map((d) => ({ ...d })),
  )
  if (result.mappings == null) {
    return { code: result.code }
  }
  return {
    code: result.code,
    map: JSON.stringify({
      version: 3,
      sources: [sourceName],
      names: [],
      mappings: result.mappings,
    }),
  }
}

const DTS_EXT_REGEX = /\.d\.([cm]?)ts$/
const TS_EXT_REGEX = /\.([cm]?)ts(x?)$/

/** Whether `fileName` is a declaration file (`.d.ts`, `.d.mts`, `.d.cts`). */
export function isDtsFile(fileName: string): boolean {
  return DTS_EXT_REGEX.test(fileName)
}

/**
 * The source ids a `.d.ts` module id can come from, the reverse of
 * rolldown-plugin-dts's `filename_to_dts`: `.d.ts` → `.ts`/`.tsx`,
 * `.d.mts` → `.mts`, `.d.cts` → `.cts`.
 */
export function dtsSourceCandidates(id: string): string[] {
  const m = DTS_EXT_REGEX.exec(id)
  if (!m) return []
  const base = id.slice(0, m.index)
  return m[1] ? [`${base}.${m[1]}ts`] : [`${base}.ts`, `${base}.tsx`]
}

/** A TS source file of the project: absolute, not a declaration, virtual or node_modules id. */
export function isProjectTsSource(id: string): boolean {
  return (
    isAbsolute(id) &&
    !id.includes('?') &&
    !/[\\/]node_modules[\\/]/.test(id) &&
    TS_EXT_REGEX.test(id) &&
    !DTS_EXT_REGEX.test(id)
  )
}

function realPathOrSelf(path: string): string {
  try {
    return realpathSync.native(path)
  } catch {
    return path
  }
}

function pathParts(path: string): string[] {
  return path.split(/[\\/]+/).filter(Boolean)
}

/** A declaration file of one output dir, see {@link matchDtsSources}. */
export interface DtsFileToMatch {
  /** The file's path in the output dir. */
  fileName: string
  /** The source its declaration map names (absolute), when it has a map. */
  mapSource?: string
}

/**
 * The graph source each declaration file of one output dir was emitted
 * from, in the order of `files`, or `undefined`.
 *
 * 1. A file with a declaration map: the source the map names, when it is in
 *    the graph, else nothing.
 * 2. Other files: tsc writes `<outDir>/<path from rootDir>.d.ts`, so one
 *    layout is found for the whole dir, a file prefix plus a source root.
 *    Each file and each graph source with the same base name and flavor
 *    (`.ts`/`.tsx` → `.d.ts`, `.mts` → `.d.mts`, `.cts` → `.d.cts`) vote
 *    for the layout that lines up the longest common end of their paths.
 *    Most votes wins; on a tie the shorter prefix. A file then matches
 *    only when it is exactly the prefix plus its source's path from the
 *    root. When layouts are still tied, a file matches only when every
 *    tied layout gives it the same source.
 *
 * So a file whose source is not in the graph (a test double the library
 * doesn't import, another package in the same dir) never gets another
 * file's members. `sourceIds` should hold every TS module of the graph, not
 * only the Angular ones, so that a plain `utils.ts` claims its own
 * `utils.d.ts`. Never matches by class name.
 */
export function matchDtsSources(
  files: readonly DtsFileToMatch[],
  sourceIds: Iterable<string>,
  caseSensitive = true,
  realPath: (path: string) => string = realPathOrSelf,
): Array<string | undefined> {
  const fold = (s: string) => (caseSensitive ? s : s.toLowerCase())
  const key = (flavor: string, parts: readonly string[]) => `${flavor}|${parts.join('/')}`

  // The graph sources by full path (for maps), by path without the
  // extension, and by base name.
  const byId = new Map<string, string>()
  const byPath = new Map<string, string>()
  const byBase = new Map<string, string[][]>()
  for (const id of sourceIds) {
    if (!isProjectTsSource(id)) continue
    const parts = pathParts(fold(id))
    byId.set(parts.join('/'), id)
    const ext = TS_EXT_REGEX.exec(id)!
    if (ext[1] && ext[2]) continue
    parts[parts.length - 1] = parts[parts.length - 1].replace(TS_EXT_REGEX, '')
    const pathKey = key(ext[1], parts)
    if (!byPath.has(pathKey)) byPath.set(pathKey, id)
    const baseKey = key(ext[1], parts.slice(-1))
    const group = byBase.get(baseKey)
    if (group) group.push(parts)
    else byBase.set(baseKey, [parts])
  }

  // A map names the source by the path the generator saw, graph ids are
  // real paths (or not, with `preserveSymlinks`): compare real paths when
  // the plain ones differ, e.g. an outDir reached through a symlink.
  let byRealId: Map<string, string> | undefined
  const byMapSource = (mapSource: string): string | undefined => {
    const hit = byId.get(pathParts(fold(mapSource)).join('/'))
    if (hit !== undefined) return hit
    if (!byRealId) {
      byRealId = new Map()
      for (const id of byId.values()) byRealId.set(pathParts(fold(realPath(id))).join('/'), id)
    }
    return byRealId.get(pathParts(fold(realPath(mapSource))).join('/'))
  }

  const result: Array<string | undefined> = Array.from({ length: files.length })
  const targets: Array<{ index: number; flavor: string; parts: string[] }> = []
  for (const [index, file] of files.entries()) {
    if (file.mapSource !== undefined) {
      result[index] = byMapSource(file.mapSource)
      continue
    }
    const m = DTS_EXT_REGEX.exec(file.fileName)
    if (m) {
      const parts = pathParts(fold(file.fileName.slice(0, m.index)))
      targets.push({ index, flavor: m[1], parts })
    }
  }

  const votes = new Map<string, { prefix: string[]; root: string[]; count: number }>()
  for (const { flavor, parts } of targets) {
    for (const source of byBase.get(key(flavor, parts.slice(-1))) ?? []) {
      let same = 1
      while (
        same < parts.length &&
        same < source.length &&
        parts[parts.length - 1 - same] === source[source.length - 1 - same]
      ) {
        same++
      }
      const prefix = parts.slice(0, parts.length - same)
      const root = source.slice(0, source.length - same)
      const voteKey = `${prefix.join('/')}\0${root.join('/')}`
      const vote = votes.get(voteKey)
      if (vote) vote.count++
      else votes.set(voteKey, { prefix, root, count: 1 })
    }
  }

  let best: Array<{ prefix: string[]; root: string[]; count: number }> = []
  for (const vote of votes.values()) {
    const top = best[0]
    if (
      !top ||
      vote.count > top.count ||
      (vote.count === top.count && vote.prefix.length < top.prefix.length)
    ) {
      best = [vote]
    } else if (vote.count === top.count && vote.prefix.length === top.prefix.length) {
      best.push(vote)
    }
  }
  if (best.length === 0) return result

  // Every tied layout must give a file the same source: twins in two dirs
  // (a `testing/` stub next to `src/lib/`) are never guessed.
  for (const { index, flavor, parts } of targets) {
    let source: string | undefined
    for (const [i, { prefix, root }] of best.entries()) {
      const inLayout = parts.length > prefix.length && prefix.every((part, j) => part === parts[j])
      const found = inLayout
        ? byPath.get(key(flavor, [...root, ...parts.slice(prefix.length)]))
        : undefined
      if (i > 0 && found !== source) {
        source = undefined
        break
      }
      source = found
    }
    result[index] = source
  }
  return result
}

const SOURCE_MAPPING_URL_REGEX = /\/\/# sourceMappingURL=(\S+)/g
const INLINE_MAP_REGEX = /^data:application\/json(?:;[^,;]*)*;base64,/

/** The new code of a `.d.ts` file, and its map file to write back, if any. */
export interface DtsFileResult {
  code: string
  /** `ref` is the map file's path, relative to the `.d.ts`'s directory. */
  map?: { ref: string; json: string }
}

/**
 * {@link injectDtsDeclarations} for a whole declaration file, keeping its
 * declaration map in step (see {@link injectDtsDeclarationsWithMap}).
 *
 * The map is found the way bundlers and tsc point at it: the last
 * `//# sourceMappingURL=` comment, either a base64 `data:application/json`
 * URL (rewritten in place in the new code) or a path; with no comment,
 * `<file>.map`. `readMap(ref)` reads a map file by its path relative to the
 * `.d.ts`'s directory, or gives `undefined`. Returns `undefined` when
 * nothing was injected.
 */
export function injectDtsFile(
  code: string,
  declarations: readonly DtsClassDeclaration[],
  fileName: string,
  readMap: (ref: string) => string | undefined,
): DtsFileResult | undefined {
  const map = locateMap(code, fileName)
  if ('url' in map) {
    const { url, inline } = map
    const result = injectDtsDeclarationsWithMap(code, declarations, map.json)
    if (result.code === code) return undefined
    const at = result.code.lastIndexOf(url)
    if (!inline || !result.map || at === -1) return { code: result.code }
    const newUrl = inline + Buffer.from(result.map, 'utf8').toString('base64')
    return { code: result.code.slice(0, at) + newUrl + result.code.slice(at + url.length) }
  }

  const { ref } = map
  const result = injectDtsDeclarationsWithMap(code, declarations, readMap(ref))
  if (result.code === code) return undefined
  return result.map ? { code: result.code, map: { ref, json: result.map } } : { code: result.code }
}

/**
 * The source a declaration file's map names: its first `sources` entry,
 * resolved against the map's location and `sourceRoot`, as tsc writes it.
 * `file` is the declaration file's absolute path; the map is found and read
 * as in {@link injectDtsFile}. `undefined` when there is no usable map.
 */
export function dtsMapSource(
  code: string,
  file: string,
  readMap: (ref: string) => string | undefined,
): string | undefined {
  const map = locateMap(code, file)
  const json = 'url' in map ? map.json : readMap(map.ref)
  let value: { sources?: unknown; sourceRoot?: unknown } | undefined
  try {
    value = json ? JSON.parse(json) : undefined
  } catch {
    return undefined
  }
  const source = Array.isArray(value?.sources) ? value.sources[0] : undefined
  if (typeof source !== 'string' || !source) return undefined
  const sourceRoot = typeof value?.sourceRoot === 'string' ? value.sourceRoot : ''
  if (/^[a-z][\w+.-]*:\/\//i.test(sourceRoot) || /^[a-z][\w+.-]*:\/\//i.test(source)) {
    return undefined
  }
  const mapDir = 'url' in map ? dirname(file) : dirname(resolve(dirname(file), map.ref))
  return resolve(mapDir, sourceRoot, source)
}

/**
 * Where a declaration file's map is: the last `//# sourceMappingURL=`
 * comment, either a `data:` URL (with the base64 JSON prefix and the JSON,
 * when it is one) or a path; with no comment, `<file>.map`.
 */
function locateMap(
  code: string,
  fileName: string,
): { url: string; inline?: string; json?: string } | { ref: string } {
  let url: string | undefined
  for (const match of code.matchAll(SOURCE_MAPPING_URL_REGEX)) url = match[1]

  if (url?.startsWith('data:')) {
    const inline = INLINE_MAP_REGEX.exec(url)?.[0]
    // Maps can hold non-ASCII names, so decode with Buffer, not `atob`.
    const json = inline
      ? Buffer.from(url.slice(inline.length), 'base64').toString('utf8')
      : undefined
    return { url, inline, json }
  }
  if (!url) return { ref: `${pathParts(fileName).pop()}.map` }
  try {
    return { ref: decodeURI(url) }
  } catch {
    return { ref: url }
  }
}

/** Every declaration file under `dir`, skipping `node_modules` and dot-dirs. */
export async function findDtsFiles(dir: string): Promise<string[]> {
  const files: string[] = []
  async function walk(current: string): Promise<void> {
    let entries: Dirent[]
    try {
      entries = await readdir(current, { withFileTypes: true })
    } catch {
      return
    }
    for (const entry of entries) {
      const full = join(current, entry.name)
      if (entry.isDirectory()) {
        if (entry.name !== 'node_modules' && !entry.name.startsWith('.')) await walk(full)
      } else if (entry.isFile() && DTS_EXT_REGEX.test(entry.name)) {
        files.push(full)
      }
    }
  }
  await walk(dir)
  return files
}

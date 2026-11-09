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
import type { Dirent } from 'node:fs'
import { readdir } from 'node:fs/promises'
import { isAbsolute, join } from 'node:path'

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

function pathParts(path: string): string[] {
  return path.split(/[\\/]+/).filter(Boolean)
}

/**
 * The source a declaration file `fileName` (its path in the output dir) was
 * emitted from, or `undefined`.
 *
 * tsc writes `<declarationDir>/<path relative to rootDir>.d.ts`, and the
 * default rootDir is the common directory of the sources (TypeScript
 * `getCommonSourceDirectory`). So a source matches when its whole path from
 * the common directory of `sourceIds`, without the extension, ends
 * `fileName`; the longest one wins. Two different paths of the same length
 * can't both end one path, so there is no tie. `sourceIds` should hold every
 * TS module of the graph, not only the Angular ones, so that a plain
 * `utils.ts` claims its own `utils.d.ts`. Never matches by class name.
 */
export function matchDtsSource(
  fileName: string,
  sourceIds: Iterable<string>,
  caseSensitive = true,
): string | undefined {
  const m = DTS_EXT_REGEX.exec(fileName)
  if (!m) return undefined
  const flavor = m[1]
  const fold = (s: string) => (caseSensitive ? s : s.toLowerCase())
  const target = pathParts(fold(fileName.slice(0, m.index)))

  const sources: Array<{ id: string; parts: string[] }> = []
  for (const id of sourceIds) {
    if (isProjectTsSource(id)) sources.push({ id, parts: pathParts(fold(id)) })
  }
  if (sources.length === 0) return undefined

  // Number of parts in the common source directory.
  const first = sources[0].parts
  let common = first.length - 1
  for (const { parts } of sources) {
    let i = 0
    while (i < common && i < parts.length - 1 && parts[i] === first[i]) i++
    common = i
  }

  let best: { id: string; length: number } | undefined
  for (const { id, parts } of sources) {
    const ext = TS_EXT_REGEX.exec(id)!
    if (ext[1] !== flavor || (flavor && ext[2])) continue
    const rel = parts.slice(common)
    rel[rel.length - 1] = rel[rel.length - 1].replace(TS_EXT_REGEX, '')
    if (rel.length > target.length || (best && rel.length <= best.length)) continue
    const offset = target.length - rel.length
    if (rel.every((part, i) => part === target[offset + i])) {
      best = { id, length: rel.length }
    }
  }
  return best?.id
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
  let url: string | undefined
  for (const match of code.matchAll(SOURCE_MAPPING_URL_REGEX)) url = match[1]

  if (url?.startsWith('data:')) {
    const inline = INLINE_MAP_REGEX.exec(url)
    // Maps can hold non-ASCII names, so decode with Buffer, not `atob`.
    const json = inline
      ? Buffer.from(url.slice(inline[0].length), 'base64').toString('utf8')
      : undefined
    const result = injectDtsDeclarationsWithMap(code, declarations, json)
    if (result.code === code) return undefined
    const at = result.code.lastIndexOf(url)
    if (!inline || !result.map || at === -1) return { code: result.code }
    const newUrl = inline[0] + Buffer.from(result.map, 'utf8').toString('base64')
    return { code: result.code.slice(0, at) + newUrl + result.code.slice(at + url.length) }
  }

  let ref = `${pathParts(fileName).pop()}.map`
  if (url) {
    try {
      ref = decodeURI(url)
    } catch {
      ref = url
    }
  }
  const result = injectDtsDeclarationsWithMap(code, declarations, readMap(ref))
  if (result.code === code) return undefined
  return result.map ? { code: result.code, map: { ref, json: result.map } } : { code: result.code }
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

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

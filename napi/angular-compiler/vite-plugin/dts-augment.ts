/**
 * Add Angular's Ivy members (`static ɵfac`, `static ɵcmp`, …) to declaration
 * files that a separate `tsc` run wrote, outside any Vite / Rolldown build.
 *
 * ngc adds these members while TypeScript writes the declarations (an
 * `afterDeclarations` transform). A plain `tsc` run has no such step, so this
 * module adds them afterwards to the files `tsc` wrote: each file gets the
 * members of its own source file, never matched by class name, with the same
 * splice code (`injectDtsFile`) the Vite plugin uses.
 *
 * This file must not import `vite`: it is the `@oxc-angular/vite/dts` entry,
 * meant for plain Node scripts.
 */
import { readFileSync } from 'node:fs'
import { readFile, stat, writeFile } from 'node:fs/promises'
import { createRequire } from 'node:module'
import { dirname, resolve } from 'node:path'

import {
  extractComponentUrls,
  transformAngularFile,
  type AngularVersion,
  type ResolvedResources,
  type TransformOptions,
} from '#binding'
import type * as TS from 'typescript'

import { installedAngularVersion, parseAngularVersion } from './utils/angular-version.js'
import { isFileSystemCaseSensitive } from './utils/case-sensitive.js'
import {
  dtsMapSource,
  findDtsFiles,
  injectDtsFile,
  isProjectTsSource,
  matchDtsSources,
  type DtsClassDeclaration,
} from './utils/dts.js'

export type { AngularVersion } from '#binding'

/** Options of {@link augmentDeclarations}. */
export interface AugmentDeclarationsOptions {
  /**
   * The tsconfig the declarations were built with (`tsc -p <tsconfig>`).
   *
   * Without `sources`, the files and output paths come from TypeScript
   * itself (the `typescript` package installed for the tsconfig's dir), so
   * every file `tsc` wrote is found, including files only reached through
   * imports. With `sources`, it is only used to compile them (`paths`
   * aliases, `extends`), and TypeScript is not loaded.
   */
  tsconfig?: string
  /**
   * Compiler options that were given to `tsc` on the command line, in
   * tsconfig form, such as `{ outDir: 'types' }` for
   * `tsc -p tsconfig.json --outDir types`. Relative paths are taken from
   * `cwd`, as `tsc` does. Only used with `tsconfig` and no `sources`.
   * `declaration` and `emitDeclarationOnly` are always on.
   */
  compilerOptions?: Record<string, unknown>
  /**
   * The dir that holds the declaration files, relative to `cwd`.
   *
   * With `tsconfig` and no `sources`: the same as
   * `compilerOptions.declarationDir`. With `sources`: the dir to scan.
   */
  declarationDir?: string
  /**
   * The source files of the library (relative to `cwd` or absolute). With
   * this, TypeScript is not loaded: each declaration file under
   * `declarationDir` is matched to one of these by its declaration map, or,
   * without maps, by the one layout `tsc` used for the whole dir. A file
   * that matches no source is left alone and listed in `unmatched`. List
   * every TS file of the library, not only the Angular ones, so a plain
   * `util.ts` claims its own `util.d.ts`.
   */
  sources?: readonly string[]
  /**
   * The Angular version to write members for, as an object or a string
   * (`'20.1.0'`). Default: the `@angular/core` installed for the tsconfig's
   * dir (or `cwd`).
   */
  angularVersion?: AngularVersion | string
  /** The dir relative paths are taken from. Default: `process.cwd()`. */
  cwd?: string
}

/** A compiler error. The declaration file of `file` was left as it was. */
export interface AugmentDeclarationsError {
  file: string
  message: string
}

/** What {@link augmentDeclarations} did. All paths are absolute. */
export interface AugmentDeclarationsResult {
  /** Declaration files that were changed (their maps were kept in step). */
  written: string[]
  /** Declaration files that were paired with a source file, changed or not. */
  matched: string[]
  /** With `sources`: declaration files whose source is not one of them. */
  unmatched: string[]
  /** Without `sources`: declaration files `tsc` would write that are not there. */
  missing: string[]
  /** Compiler errors; the declaration files of these sources were not touched. */
  errors: AugmentDeclarationsError[]
}

/** One declaration file and the source it was written from. */
interface Target {
  file: string
  source: string
  /**
   * Without `sources`: what `tsc` writes for the file now (the text, and
   * the map file with its text), from an in-memory declaration emit.
   */
  emitted?: { code: string; mapFile?: string; map?: string }
}

/** The options each source file is compiled with. */
interface CompileContext {
  tsconfigPath?: string
  rootDirs: string[]
  caseSensitive: boolean
  angularVersion?: AngularVersion
}

/**
 * Add Angular's Ivy members to the declaration files a `tsc` run wrote:
 *
 * ```ts
 * import { augmentDeclarations } from '@oxc-angular/vite/dts'
 *
 * // after `tsc -p tsconfig.lib.json`
 * const result = await augmentDeclarations({ tsconfig: 'tsconfig.lib.json' })
 * if (result.errors.length > 0) process.exitCode = 1
 * ```
 *
 * Each declaration file gets the members of its own source file, plus the
 * `import * as i0 from "@angular/core"` imports they use, and its
 * declaration map is shifted to fit. Running it twice changes nothing the
 * second time. A source that fails to compile is reported in `errors` and
 * its declaration file is left alone.
 */
export async function augmentDeclarations(
  options: AugmentDeclarationsOptions,
): Promise<AugmentDeclarationsResult> {
  const cwd = resolve(options.cwd ?? process.cwd())
  const tsconfig = options.tsconfig === undefined ? undefined : resolve(cwd, options.tsconfig)
  const angularVersion =
    typeof options.angularVersion === 'string'
      ? parseAngularVersion(options.angularVersion)
      : options.angularVersion
  if (typeof options.angularVersion === 'string' && !angularVersion) {
    throw new Error(`augmentDeclarations: "${options.angularVersion}" is not a version.`)
  }

  const result: AugmentDeclarationsResult = {
    written: [],
    matched: [],
    unmatched: [],
    missing: [],
    errors: [],
  }

  let targets: Target[]
  let context: CompileContext
  if (options.sources) {
    if (options.declarationDir === undefined) {
      throw new Error('augmentDeclarations: `sources` needs `declarationDir`.')
    }
    const caseSensitive = isFileSystemCaseSensitive(cwd)
    targets = await matchedTargets(
      resolve(cwd, options.declarationDir),
      options.sources.map((source) => resolve(cwd, source)),
      caseSensitive,
      result,
    )
    context = {
      tsconfigPath: tsconfig,
      rootDirs: [cwd],
      caseSensitive,
      angularVersion: angularVersion ?? installedAngularVersion(tsconfig ? dirname(tsconfig) : cwd),
    }
  } else if (tsconfig) {
    const ts = loadTypeScript(tsconfig)
    const project = emittedTargets(ts, tsconfig, cwd, options)
    targets = project.targets
    result.errors.push(...project.errors)
    context = {
      tsconfigPath: tsconfig,
      rootDirs: [project.rootDir ?? dirname(tsconfig)],
      caseSensitive: ts.sys.useCaseSensitiveFileNames,
      angularVersion: angularVersion ?? installedAngularVersion(dirname(tsconfig)),
    }
  } else {
    throw new Error('augmentDeclarations: pass `tsconfig`, or `declarationDir` and `sources`.')
  }

  for (const target of targets) {
    await augmentTarget(target, context, result)
  }
  return result
}

/**
 * The `typescript` package installed for the tsconfig's dir (the one `tsc`
 * came from), else the one this package can see.
 */
function loadTypeScript(tsconfig: string): typeof TS {
  for (const from of [tsconfig, import.meta.url]) {
    try {
      return createRequire(from)('typescript') as typeof TS
    } catch {
      // try the next place
    }
  }
  throw new Error(
    `augmentDeclarations: the \`typescript\` package was not found from ${dirname(tsconfig)}. ` +
      'Install it, or pass `declarationDir` and `sources` instead.',
  )
}

/**
 * Every declaration file `tsc -p tsconfig` writes, with its source file,
 * from a declaration emit of the whole program that writes nothing. The
 * program holds the files reached through imports too, and the emit lays
 * them out exactly as `tsc` does (common source dir, `declarationDir`,
 * `.d.mts`, `mapRoot`, `stripInternal`).
 */
function emittedTargets(
  ts: typeof TS,
  tsconfig: string,
  cwd: string,
  options: AugmentDeclarationsOptions,
): { targets: Target[]; errors: AugmentDeclarationsError[]; rootDir?: string } {
  const formatHost: TS.FormatDiagnosticsHost = {
    getCanonicalFileName: (fileName) => fileName,
    getCurrentDirectory: () => cwd,
    getNewLine: () => '\n',
  }
  const describe = (diagnostics: readonly TS.Diagnostic[]) =>
    ts.formatDiagnostics(diagnostics, formatHost).trim()

  const json: Record<string, unknown> = { ...options.compilerOptions }
  if (options.declarationDir !== undefined) json.declarationDir = options.declarationDir
  const converted = ts.convertCompilerOptionsFromJson(json, cwd)
  if (converted.errors.length > 0) {
    throw new Error(`augmentDeclarations: ${describe(converted.errors)}`)
  }
  const optionsToExtend: TS.CompilerOptions = {
    ...converted.options,
    declaration: true,
    emitDeclarationOnly: true,
    noEmit: false,
    noEmitOnError: false,
  }

  const host: TS.ParseConfigFileHost = {
    ...ts.sys,
    getCurrentDirectory: () => cwd,
    onUnRecoverableConfigFileDiagnostic(diagnostic) {
      throw new Error(`augmentDeclarations: ${describe([diagnostic])}`)
    },
  }
  const config = ts.getParsedCommandLineOfConfigFile(tsconfig, optionsToExtend, host)
  if (!config) throw new Error(`augmentDeclarations: cannot read ${tsconfig}.`)
  if (config.options.outFile) {
    throw new Error(
      'augmentDeclarations: `outFile` (one bundled declaration file) is not supported.',
    )
  }
  const errors = config.errors
    .filter((d) => d.category === ts.DiagnosticCategory.Error)
    .map((d) => ({ file: tsconfig, message: describe([d]) }))

  const program = ts.createProgram({
    rootNames: config.fileNames,
    options: config.options,
    projectReferences: config.projectReferences,
  })
  const bySource = new Map<string, Target>()
  program.emit(
    undefined,
    (fileName, text, writeByteOrderMark, _onError, sourceFiles) => {
      // `sourceFiles` holds the transformed declaration nodes; their
      // `fileName` is still the source file's.
      const emittedFrom = sourceFiles?.length === 1 ? sourceFiles[0] : undefined
      const sourceFile = emittedFrom && program.getSourceFile(emittedFrom.fileName)
      if (!sourceFile || sourceFile.isDeclarationFile) return
      if (program.isSourceFileFromExternalLibrary(sourceFile)) return
      let target = bySource.get(sourceFile.fileName)
      if (!target) {
        target = { file: '', source: resolve(sourceFile.fileName), emitted: { code: '' } }
        bySource.set(sourceFile.fileName, target)
      }
      const code = writeByteOrderMark ? BOM + text : text
      if (fileName.endsWith('.map')) {
        target.emitted!.mapFile = resolve(cwd, fileName)
        target.emitted!.map = code
      } else {
        target.file = resolve(cwd, fileName)
        target.emitted!.code = code
      }
    },
    undefined,
    /* emitOnlyDtsFiles */ true,
  )
  const targets = [...bySource.values()].filter(
    (target) => target.file && isProjectTsSource(target.source),
  )
  return { targets, errors, rootDir: config.options.rootDir }
}

/**
 * The declaration files under `dir`, each paired with its source among
 * `sources` the way the Vite plugin pairs the files a generator wrote:
 * by declaration map, else by the one layout of the dir.
 */
async function matchedTargets(
  dir: string,
  sources: string[],
  caseSensitive: boolean,
  result: AugmentDeclarationsResult,
): Promise<Target[]> {
  const found = (await stat(dir).catch(() => undefined))?.isDirectory()
    ? await findDtsFiles(dir)
    : undefined
  // A wrong or not yet built dir must not pass as "nothing to do".
  if (!found) throw new Error(`augmentDeclarations: declarationDir ${dir} is not a directory.`)
  if (found.length === 0 && sources.length > 0) {
    throw new Error(`augmentDeclarations: declarationDir ${dir} holds no declaration files.`)
  }
  const files: Array<{ fileName: string; mapSource?: string }> = []
  for (const file of found) {
    const code = await readFile(file, 'utf8')
    const mapSource = dtsMapSource(code, file, (ref) => readTextSync(resolve(dirname(file), ref)))
    files.push({ fileName: file, mapSource })
  }
  const matched = matchDtsSources(files, sources, caseSensitive)
  const targets: Target[] = []
  for (const [i, { fileName }] of files.entries()) {
    const source = matched[i]
    if (source) targets.push({ file: fileName, source })
    else result.unmatched.push(fileName)
  }
  return targets
}

/** Add the members of `target.source` to `target.file`, when they are new. */
async function augmentTarget(
  target: Target,
  context: CompileContext,
  result: AugmentDeclarationsResult,
): Promise<void> {
  const { file, emitted } = target
  const disk = await readText(file)
  if (disk === undefined) {
    result.missing.push(file)
    return
  }
  result.matched.push(file)

  const declarations = await compile(target.source, context, result.errors)
  if (!declarations) return

  // Start from what `tsc` writes now when the file on disk is that text,
  // or that text plus Ivy members from an earlier run: an incremental `tsc`
  // leaves a file alone when its own text did not change, even when its
  // members did (a selector held in another file). Anything else on disk
  // (`tsc` run with other options than these) is patched as it is, and
  // members already in it are kept.
  let base = disk
  let mapFile: string | undefined
  let baseMap: string | undefined
  // Whether `base` is the text `tsc` writes for these options.
  let fresh = false
  if (emitted) {
    mapFile = emitted.mapFile
    if (disk === emitted.code) {
      // Keep the map `tsc` wrote: its paths are spelled as `tsc` saw them.
      fresh = true
      baseMap = (mapFile && (await readText(mapFile))) ?? emitted.map
    } else if (sameApartFromIvyMembers(disk, emitted.code)) {
      fresh = true
      base = emitted.code
      baseMap = emitted.map
    } else if (mapFile) {
      baseMap = await readText(mapFile)
    }
  }

  // The splice works on `\n` lines. tsc ends lines with `\r\n` under
  // `newLine: "crlf"`, but copies the line breaks inside a template literal
  // type from the source as they are. So the line break tsc used is the
  // last one in the file, and only the lines the splice adds get it. Lines
  // stay where they are, so the map is the same either way.
  const bom = base.startsWith(BOM) ? BOM : ''
  let code = base.slice(bom.length)
  const lastBreak = code.lastIndexOf('\n')
  const crlf = lastBreak > 0 && code[lastBreak - 1] === '\r'
  // Marks a `\n` that is not part of a `\r\n`, so it stays a lone `\n`.
  const mark = crlf && !code.includes(LONE_LF_MARK)
  if (crlf) {
    if (mark) code = code.replace(/(?<!\r)\n/g, `${LONE_LF_MARK}\n`)
    code = code.replace(/\r\n/g, '\n')
  }
  let map = baseMap
  if (declarations.length > 0) {
    // With a known map file (`mapRoot` makes the comment a URL), use it;
    // else follow the file's own `//# sourceMappingURL=` comment.
    const readMap = (ref: string) =>
      fresh || mapFile ? baseMap : readTextSync(resolve(dirname(file), ref))
    const injected = injectDtsFile(code, declarations, file, readMap)
    if (injected) {
      code = injected.code
      if (injected.map) {
        map = injected.map.json
        mapFile ??= resolve(dirname(file), injected.map.ref)
      }
    }
  }
  if (crlf) {
    code = mark
      ? code.replace(/(?<!\0)\n/g, '\r\n').replace(/\0\n/g, '\n')
      : code.replace(/\n/g, '\r\n')
  }
  code = bom + code

  // A file that stays as it is keeps its map as it is too.
  if (code === disk) return
  await writeFile(file, code)
  result.written.push(file)
  if (mapFile && map !== undefined && map !== (await readText(mapFile))) {
    await writeFile(mapFile, map)
  }
}

/**
 * The `.d.ts` members of the Angular classes in `source`, compiled as the
 * Vite plugin compiles a library (`compilationMode: 'partial'`), or
 * `undefined` when the compiler reports errors (pushed to `errors`).
 */
async function compile(
  source: string,
  context: CompileContext,
  errors: AugmentDeclarationsError[],
): Promise<DtsClassDeclaration[] | undefined> {
  let code: string
  try {
    code = await readFile(source, 'utf8')
  } catch (e) {
    errors.push({ file: source, message: `cannot read the file: ${(e as Error).message}` })
    return undefined
  }
  // Ivy members only go on decorated classes.
  if (!code.includes('@')) return []

  const options: TransformOptions = {
    compilationMode: 'partial',
    angularVersion: context.angularVersion,
    rootDirs: context.rootDirs,
    useCaseSensitiveFileNames: context.caseSensitive,
    resolveImportedValues: true,
    baseDir: dirname(source),
    tsconfigPath: context.tsconfigPath,
  }

  // Templates are read as they are, so they parse and their `<link>`
  // stylesheets are found. No member uses style text, so styles stay empty.
  // Partial mode writes `never` for `ngContentSelectors`, where ngc lists them.
  const { templateUrls, styleUrls } = await extractComponentUrls(code, source, options)
  const templates: Record<string, string> = {}
  for (const url of templateUrls) {
    const text = await readText(resolve(dirname(source), url))
    if (text !== undefined) templates[url] = text
  }
  const styles: Record<string, string[]> = {}
  for (const url of styleUrls) styles[url] = ['']
  const resources = { templates, styles } as unknown as ResolvedResources

  let output = await transformAngularFile(code, source, options, resources)
  // A template `<link rel="stylesheet">` is one more stylesheet.
  if (output.missingLinkUrls.length > 0) {
    for (const url of output.missingLinkUrls) styles[url] = ['']
    output = await transformAngularFile(code, source, options, resources)
  }
  if (output.errors.length > 0) {
    for (const error of output.errors) {
      errors.push({
        file: source,
        message: error.helpMessage ? `${error.message}\n${error.helpMessage}` : error.message,
      })
    }
    return undefined
  }
  return output.dtsDeclarations.map((decl) => ({
    className: decl.className,
    members: decl.members,
    namespaceImports: decl.namespaceImports,
    sourceFile: source,
  }))
}

// A byte order mark, which `tsc` writes with `emitBOM`.
const BOM = String.fromCharCode(0xfeff)

// See `augmentTarget`: put in front of a lone `\n` while the splice runs.
const LONE_LF_MARK = '\0'

// The lines the splice adds: Ivy statics, and the namespace imports they use.
const IVY_MEMBER_LINE =
  /^\s*static (?:ɵ(?:fac|cmp|dir|pipe|mod|inj|prov)|ngAcceptInputType_[^\s:?]+)\??:/
const NAMESPACE_IMPORT_LINE = /^import \* as i\d+(?:_\d+)* from (["']).*\1;\r?$/

/**
 * Whether `a` and `b` are the same text once the lines the splice adds are
 * removed. Line breaks must match too: `\r\n` is not the same as `\n`.
 */
function sameApartFromIvyMembers(a: string, b: string): boolean {
  const strip = (text: string) =>
    text
      .split('\n')
      .filter((line) => !IVY_MEMBER_LINE.test(line) && !NAMESPACE_IMPORT_LINE.test(line))
      .join('\n')
  return strip(a) === strip(b)
}

async function readText(file: string): Promise<string | undefined> {
  try {
    return await readFile(file, 'utf8')
  } catch {
    return undefined
  }
}

function readTextSync(file: string): string | undefined {
  try {
    return readFileSync(file, 'utf8')
  } catch {
    return undefined
  }
}

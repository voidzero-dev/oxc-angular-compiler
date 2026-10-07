/**
 * Inject Angular's Ivy `.d.ts` type declarations into emitted declaration
 * files for library builds.
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
 * `./other` in another), so this pass canonicalizes: one alias per module
 * identity, collision-free against every identifier in the emitted file.
 * The emitted `.d.ts` is parsed once with the oxc parser; class bodies are
 * located through the AST (so type-parameter constraints containing `{` are
 * no issue) and member alias heads are rewritten by splicing the
 * `TSQualifiedName` leftmost identifier the parser reports.
 */
import {
  parseSync,
  Visitor,
  type ClassBody,
  type Directive,
  type Program,
  type Statement,
} from 'oxc-parser'

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

/** One namespace-alias head found in a member body. */
interface MemberHead {
  /** `(start, end)` of the head inside the member's own text. */
  start: number
  end: number
  /** The head's alias as compiled (what `text.slice(start, end)` reads). */
  alias: string
  /** The module specifier the alias refers to — emitted verbatim, matching
   * ngtsc and structure-preserving declaration emit (`src/a/foo.ts` →
   * `dist/a/foo.d.ts`, where `"./dep"` reaches `dist/a/dep.d.ts`). */
  module: string
  /** The canonical identity for deduping — the specifier resolved against
   * the declaration's source file, so `"./dep"` under different directories
   * is two modules, not one. */
  resolved: string
}

/**
 * The alias → specifier map of every `import * as ns` the file already has.
 */
function namespaceImportsOf(program: Program): Map<string, string> {
  const aliases = new Map<string, string>()
  for (const stmt of program.body) {
    if (stmt.type !== 'ImportDeclaration') {
      continue
    }
    for (const spec of stmt.specifiers) {
      if (spec.type === 'ImportNamespaceSpecifier') {
        aliases.set(spec.local.name, String(stmt.source.value))
      }
    }
  }
  return aliases
}

/** Every identifier name in the file (`Identifier` covers all flavors). */
function collectIdentifiers(program: Program): Set<string> {
  const names = new Set<string>()
  new Visitor({
    Identifier(node) {
      names.add(node.name)
    },
  }).visit(program)
  return names
}

/**
 * `i0`, `i0_1`, `i0_2`, … — the first name not colliding with `names`.
 * Matches `check_unique_identifier_name` in ngtsc's ImportManager.
 */
function uniquifyIdentifier(base: string, names: Set<string>): string {
  if (!names.has(base)) {
    return base
  }
  let counter = 1
  while (names.has(`${base}_${counter}`)) {
    counter += 1
  }
  return `${base}_${counter}`
}

/**
 * The `ClassBody` of the class named `className`, searching through export
 * wrappers and ambient module blocks (the latter covers `declare module`
 * declarations a bundler may wrap library classes in).
 */
function findClassBody(
  statements: ReadonlyArray<Directive | Statement>,
  className: string,
): ClassBody | null {
  for (const stmt of statements) {
    // `export declare class`, `export default class`, `export declare module`.
    const node =
      stmt.type === 'ExportNamedDeclaration' || stmt.type === 'ExportDefaultDeclaration'
        ? stmt.declaration
        : stmt
    if (node === null) {
      continue
    }
    if (node.type === 'ClassDeclaration' && node.id?.name === className) {
      return node.body
    }
    if (node.type === 'TSModuleDeclaration' && node.body?.type === 'TSModuleBlock') {
      const found = findClassBody(node.body.body, className)
      if (found !== null) {
        return found
      }
    }
  }
  return null
}

/** Every non-computed STATIC member name in `body`, as written. Instance
 * members don't count: TypeScript permits `ɵfac` and `static ɵfac` to
 * coexist, and only a previously generated static is idempotent. */
function existingMemberNames(body: ClassBody): Set<string> {
  const names = new Set<string>()
  for (const element of body.body) {
    const member = element as {
      computed?: boolean
      static?: boolean
      key?: { type: string; name?: string; value?: unknown }
    }
    if (member.static !== true || member.computed || member.key === undefined) {
      continue
    }
    if (member.key.type === 'Identifier' && member.key.name !== undefined) {
      names.add(member.key.name)
    } else if (member.key.type === 'Literal') {
      names.add(String(member.key.value))
    }
  }
  return names
}

/**
 * Parse `members` (wrapped in a dummy `declare class`) into per-member
 * entries: the member's name, its exact text slice, and its namespace heads.
 * Each head is the leftmost `Identifier` of a `TSQualifiedName` (`i0.ɵɵX`
 * and `typeof i1.SomeDirective` alike — a nested `a.b.c` chain is visited as
 * its inner `a.b` too, so the head is always found).
 *
 * Returns `null` when the member text doesn't parse; the caller skips the
 * declaration rather than splice members whose aliases might be wrong.
 */
function parseMembers(
  members: string,
  namespaceImports: Readonly<Record<string, string>>,
  resolve: (specifier: string) => string,
): { name: string | null; text: string; heads: MemberHead[] }[] | null {
  const prefix = 'declare class X {\n'
  const wrapped = prefix + members + '\n}'
  const { program, errors } = parseSync('members.d.ts', wrapped, { lang: 'dts' })
  if (errors.length > 0) {
    return null
  }
  const cls = program.body[0]
  if (cls === undefined || cls.type !== 'ClassDeclaration') {
    return []
  }
  // Collect every namespace head in one program-wide pass (the Visitor only
  // accepts a Program), binned into the member that contains it.
  const collected: {
    pos: number
    alias: string
    module: string
    resolved: string
  }[] = []
  new Visitor({
    TSQualifiedName(node) {
      if (node.left.type !== 'Identifier') {
        return
      }
      const alias = node.left.name
      const module = namespaceImports[alias]
      if (module === undefined && !node.right.name.startsWith('ɵ')) {
        // Not a generated namespace reference — resolve against whatever
        // the `.d.ts` already imports and leave the head untouched.
        return
      }
      const specifier = module ?? '@angular/core'
      collected.push({
        pos: node.left.start - prefix.length,
        alias,
        module: specifier,
        resolved: resolve(specifier),
      })
    },
  }).visit(program)

  return cls.body.body.map((element) => {
    // Spans are relative to `wrapped`; shift them back into `members`.
    const start = element.start - prefix.length
    const end = element.end - prefix.length
    const member = element as {
      computed?: boolean
      key?: { type: string; name?: string; value?: unknown }
    }
    const name = member.computed
      ? null
      : member.key === undefined
        ? null
        : member.key.type === 'Identifier'
          ? (member.key.name ?? null)
          : member.key.type === 'Literal'
            ? String(member.key.value)
            : null
    const heads: MemberHead[] = collected
      .filter((head) => head.pos >= start && head.pos < end)
      .map((head) => ({
        start: head.pos - start,
        end: head.pos - start + head.alias.length,
        alias: head.alias,
        module: head.module,
        resolved: head.resolved,
      }))
    return { name, text: members.slice(start, end), heads }
  })
}

/**
 * The position just past `position`'s line, skipping newlines that fall
 * inside comments — `parseSync`'s `comments` carry spans for both `//` and
 * `/* ... *\/` so a multiline trailing comment can't split the insertion.
 */
function endOfLineAfter(
  source: string,
  position: number,
  comments: ReadonlyArray<{ start: number; end: number }>,
): number {
  let cursor = position
  for (;;) {
    const newline = source.indexOf('\n', cursor)
    const end = newline === -1 ? source.length : newline
    const covering = comments.find((c) => c.start <= end && end < c.end)
    if (covering === undefined) {
      return newline === -1 ? source.length : newline + 1
    }
    cursor = covering.end
  }
}

/**
 * Resolve `dir + specifier` the way Node would see it — collapsing `..` and
 * `.` segments — so `"./dep"` under different directories canonicalizes to
 * different identities while `../dep` from a nested file resolves back to
 * the same one. Purely textual; no filesystem access.
 */
function resolvePath(path: string): string {
  const out: string[] = []
  for (const segment of path.split('/')) {
    if (segment === '' || segment === '.') {
      continue
    }
    if (segment === '..') {
      out.pop()
    } else {
      out.push(segment)
    }
  }
  return out.join('/')
}

/**
 * Splice each declaration's static members into the matching class body in
 * `source`, and emit the namespace imports the injected members reference —
 * one `import * as ns from "module"` per module identity, canonicalized
 * across all injected declarations.
 *
 * Like ngtsc's declaration ImportManager, an existing `import * as ns` is
 * reused only when it already binds the same alias and specifier — never
 * for a different one, and never to borrow the user's `@angular/core`
 * import: the members' compiled alias is kept when free, otherwise
 * uniquified (`i0_1`, …).
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
  if (declarations.length === 0) {
    return source
  }

  const { program, errors, comments } = parseSync('file.d.ts', source, { lang: 'dts' })
  if (errors.length > 0) {
    return source
  }

  const fileNames = collectIdentifiers(program)
  const existingImports = namespaceImportsOf(program)

  // Identity resolution is relative to each declaration's source module —
  // the same "./dep" in two directories is two modules. Emitted specifiers
  // stay verbatim, matching ngtsc and structure-preserving declaration
  // emit (`src/a/foo.ts` → `dist/a/foo.d.ts`, where "./dep" reaches
  // `dist/a/dep.d.ts`); there is no dts-generator source→output mapping to
  // rebase against, so under fully-bundled declaration output two same-named
  // relative modules remain a documented limitation.
  const resolveFor = (sourceFile?: string) => {
    if (sourceFile === undefined) {
      return (specifier: string) => specifier
    }
    const dir = sourceFile.replace(/[^/\\]*$/, '')
    return (specifier: string) =>
      specifier.startsWith('./') || specifier.startsWith('../')
        ? resolvePath(dir + specifier)
        : specifier
  }

  // Canonical module identity → [rawSpecifier, alias] for this file.
  const canonical = new Map<string, [string, string]>()
  const canonicalAlias = (head: MemberHead): string => {
    const known = canonical.get(head.resolved)
    if (known !== undefined) {
      return known[1]
    }
    const preferred = head.module === '@angular/core' ? 'i0' : head.alias
    // Reuse an existing import only when it binds this alias AND specifier —
    // same module under the same name (which is exactly what an idempotent
    // re-run sees on its second pass), never a different module.
    const existing = existingImports.get(preferred)
    const alias =
      existing !== undefined && existing === head.module
        ? preferred
        : uniquifyIdentifier(preferred, fileNames)
    canonical.set(head.resolved, [head.module, alias])
    fileNames.add(alias)
    return alias
  }

  const splices: { start: number; end: number; text: string }[] = []

  for (const declaration of declarations) {
    const members = parseMembers(
      declaration.members,
      declaration.namespaceImports ?? {},
      resolveFor(declaration.sourceFile),
    )
    if (members === null || members.length === 0) {
      continue
    }

    const body = findClassBody(program.body, declaration.className)
    if (body === null) {
      continue
    }
    const existing = existingMemberNames(body)

    // Per-member idempotency: skip members the class already declares.
    // Names — not normalized text — decide, so a re-run whose canonical
    // alias differs (the first pass's import now occupies `i0`) doesn't
    // inject the same `static ɵfac` a second time.
    const lines: string[] = []
    for (const member of members) {
      if (member.name !== null && existing.has(member.name)) {
        continue
      }
      let text = member.text
      for (const head of member.heads.sort((a, b) => b.start - a.start)) {
        const alias = canonicalAlias(head)
        text = text.slice(0, head.start) + alias + text.slice(head.end)
      }
      for (const line of text.split('\n')) {
        const trimmed = line.trim()
        if (trimmed.length > 0) {
          lines.push(trimmed)
        }
      }
    }
    if (lines.length === 0) {
      continue
    }

    // `ClassBody.end` is just past the closing brace; inserting before it
    // appends the members, matching upstream's `[...members, ...newMembers]`
    // (declaration.ts) — a class that already has members (constructor
    // overloads, declared fields) keeps them first.
    const insertAt = body.end - 1
    const needsLeadingNl = insertAt > 0 && source[insertAt - 1] !== '\n'
    splices.push({
      start: insertAt,
      end: insertAt,
      text: (needsLeadingNl ? '\n' : '') + lines.map((line) => `    ${line}`).join('\n') + '\n',
    })
  }

  if (splices.length === 0) {
    return source
  }

  // Emit an import for every canonical module the file doesn't already
  // have, sorted by alias — the same order the compiler's own namespace
  // registry generates them in.
  const importLines = [...canonical.values()]
    .filter(([specifier, alias]) => existingImports.get(alias) !== specifier)
    .sort(([, a], [, b]) => (a < b ? -1 : a > b ? 1 : 0))
    .map(([specifier, alias]) => `import * as ${alias} from "${specifier}";`)

  if (importLines.length > 0) {
    const text = importLines.join('\n')
    let lastImportEnd: number | null = null
    for (const stmt of program.body) {
      if (stmt.type === 'ImportDeclaration') {
        lastImportEnd = stmt.end
      }
    }
    if (lastImportEnd !== null) {
      // End of the last import's line, skipping multiline trailing comments
      // (`import x from "m"; /* keep\nme */`) so the comment keeps its line
      // and the new import lands on the next one.
      const position = endOfLineAfter(source, lastImportEnd, comments)
      splices.push({ start: position, end: position, text: `${text}\n` })
    } else if (program.body.length > 0) {
      // Before the first statement keeps leading comments and triple-slash
      // reference directives at the top of the file.
      splices.push({
        start: program.body[0].start,
        end: program.body[0].start,
        text: `${text}\n`,
      })
    } else {
      splices.push({
        start: source.length,
        end: source.length,
        text: `${source.endsWith('\n') || source === '' ? '' : '\n'}${text}\n`,
      })
    }
  }

  let output = source
  for (const { start, end, text } of splices.sort((a, b) => b.start - a.start)) {
    output = output.slice(0, start) + text + output.slice(end)
  }
  return output
}

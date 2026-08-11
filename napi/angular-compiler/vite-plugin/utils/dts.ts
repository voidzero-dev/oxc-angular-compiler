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
 * the Angular members into those already-generated `.d.ts`, and ensures the
 * `i0` namespace import the members reference is present.
 *
 * Known limitation: class bodies are located with a regex that stops at the
 * first `{` after `class <Name>`. A `{` inside a type-parameter constraint or
 * default (e.g. `class C<T extends { a: 1 }>`) would be mistaken for the body
 * brace. Emitted library declarations for Angular classes don't use such
 * generics, so this is accepted in exchange for not pulling in a parser.
 */

/** A single class's `.d.ts` static member declarations. */
export interface DtsClassDeclaration {
  /** The class the members belong to. */
  className: string
  /** Newline-separated `static …;` member declarations, `i0`-prefixed. */
  members: string
}

const I0_IMPORT = 'import * as i0 from "@angular/core";'

const I0_IMPORT_RE = /import\s+\*\s+as\s+i0\s+from\s+['"]@angular\/core['"]/

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
}

/**
 * Insert the `i0` namespace import, keeping it after any leading triple-slash
 * reference directives and leading comments (which must stay at the top of a
 * `.d.ts`).
 */
function ensureI0Import(source: string): string {
  if (I0_IMPORT_RE.test(source)) {
    return source
  }

  const lines = source.split('\n')
  let insertAt = 0
  for (let i = 0; i < lines.length; i++) {
    const trimmed = lines[i].trim()
    if (
      trimmed === '' ||
      trimmed.startsWith('///') ||
      trimmed.startsWith('//') ||
      trimmed.startsWith('/*') ||
      trimmed.startsWith('*')
    ) {
      insertAt = i + 1
      continue
    }
    break
  }

  lines.splice(insertAt, 0, I0_IMPORT)
  return lines.join('\n')
}

/**
 * Splice each declaration's static members into the matching class body in
 * `source`, and ensure the `i0` import is present when anything was injected.
 *
 * The pass is idempotent: a declaration whose first member already appears in
 * `source` is skipped, so re-running over an already-augmented file is a
 * no-op. A declaration whose class isn't found is silently skipped.
 */
export function injectDtsDeclarations(
  source: string,
  declarations: readonly DtsClassDeclaration[],
): string {
  if (declarations.length === 0) {
    return source
  }

  let output = source
  let injected = false

  for (const { className, members } of declarations) {
    const memberLines = members
      .split('\n')
      .map((line) => line.trim())
      .filter(Boolean)
    if (memberLines.length === 0) {
      continue
    }

    // Idempotency: if the members are already present, don't inject again.
    if (output.includes(memberLines[0])) {
      continue
    }

    // Match `(export )?(declare )?(abstract )?class <Name> …{`, capturing up
    // to and including the opening brace of the class body.
    const classBodyOpen = new RegExp(
      `(?:export\\s+)?(?:declare\\s+)?(?:abstract\\s+)?class\\s+${escapeRegExp(
        className,
      )}\\b[^{]*\\{`,
    )
    const match = classBodyOpen.exec(output)
    if (!match) {
      continue
    }

    const insertAt = match.index + match[0].length
    const body = '\n' + memberLines.map((line) => `    ${line}`).join('\n')
    output = output.slice(0, insertAt) + body + output.slice(insertAt)
    injected = true
  }

  return injected ? ensureI0Import(output) : output
}

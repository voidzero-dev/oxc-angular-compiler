import { diffLines } from 'diff'
import { parseSync, type StaticImport, type StaticExport } from 'oxc-parser'
import { format as formatWithOxfmt, type FormatOptions } from 'oxfmt'

import type { FunctionLevelComparison, FunctionDiff, AstDiff, ClassMetadataDiff } from './types.js'

/**
 * Result of comparing full-file outputs semantically.
 */
export interface FullFileCompareResult {
  /** Whether the two pieces of code are semantically equivalent */
  match: boolean
  /** Import comparison differences */
  importDiffs?: ImportDiff[]
  /** Export comparison differences */
  exportDiffs?: ExportDiff[]
  /** Class definition differences */
  classDiffs?: ClassDiff[]
  /** Static field assignment differences */
  staticFieldDiffs?: StaticFieldDiff[]
  /** Class metadata (setClassMetadata) differences */
  classMetadataDiffs?: ClassMetadataDiff[]
  /** Function-level comparison details (for template functions, etc.) */
  functionComparison?: FunctionLevelComparison
  /** Parse errors if any */
  parseErrors?: string[]
}

/**
 * Difference in import statements between Oxc and TS outputs.
 */
export interface ImportDiff {
  /** Type of difference */
  type: 'missing' | 'extra' | 'different'
  /** Module source (e.g., "@angular/core") */
  moduleSource: string
  /** Expected import specifiers (from TS) */
  expected?: string[]
  /** Actual import specifiers (from Oxc) */
  actual?: string[]
}

/**
 * Difference in export statements between Oxc and TS outputs.
 */
export interface ExportDiff {
  /** Type of difference */
  type: 'missing' | 'extra' | 'different'
  /** Exported name */
  exportName: string
  /** Expected export details (from TS) */
  expected?: string
  /** Actual export details (from Oxc) */
  actual?: string
}

/**
 * Difference in class definitions.
 */
export interface ClassDiff {
  /** Class name */
  className: string
  /** Type of difference */
  type: 'missing' | 'extra' | 'different'
  /** Description of the difference */
  description: string
}

/**
 * Difference in static field assignments (e.g., ClassName.ɵcmp = ...).
 */
export interface StaticFieldDiff {
  /** Class name */
  className: string
  /** Field name (e.g., "ɵcmp", "ɵfac") */
  fieldName: string
  /** Type of difference */
  type: 'missing' | 'extra' | 'different'
  /** Expected value (from TS) */
  expected?: string
  /** Actual value (from Oxc) */
  actual?: string
}

/**
 * Properties to ignore when normalizing AST nodes for comparison.
 * These properties contain cosmetic/location information that doesn't affect semantics.
 */
const IGNORED_PROPERTIES = new Set([
  'span',
  'start',
  'end',
  'loc',
  'range',
  'leadingComments',
  'trailingComments',
  'innerComments',
  'comments',
  'raw',
  'extra',
])

/**
 * Rewrite syntax that has the same meaning into one form:
 *
 * - `(expr)` becomes `expr`
 * - an untagged template literal without substitutions (`\`a\``) becomes the string literal
 *   with the same value (`"a"`). Tagged templates keep their form: the tag sees the raw strings.
 */
function unwrapParenthesized(node: unknown, tagged = false): unknown {
  if (node === null || typeof node !== 'object') {
    return node
  }

  const obj = node as Record<string, unknown>

  // Unwrap ParenthesizedExpression
  if (obj.type === 'ParenthesizedExpression' && 'expression' in obj) {
    return unwrapParenthesized(obj.expression)
  }

  const quasis = obj.quasis as Array<{ value: { cooked: unknown } }> | undefined
  if (
    !tagged &&
    obj.type === 'TemplateLiteral' &&
    quasis?.length === 1 &&
    (obj.expressions as unknown[]).length === 0
  ) {
    return { type: 'Literal', value: quasis[0].value.cooked }
  }

  // Recursively process arrays
  if (Array.isArray(node)) {
    return node.map((item) => unwrapParenthesized(item))
  }

  // Recursively process object properties
  const result: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(obj)) {
    result[key] = unwrapParenthesized(
      value,
      obj.type === 'TaggedTemplateExpression' && key === 'quasi',
    )
  }
  return result
}

/**
 * Normalize AST to a canonical JSON string representation.
 * This excludes location and cosmetic properties, and rewrites equivalent syntax into one form
 * (see `unwrapParenthesized`).
 */
function normalizeAst(node: unknown): string {
  // First unwrap parenthesized expressions
  const unwrapped = unwrapParenthesized(node)
  return JSON.stringify(unwrapped, (key, value) => {
    if (IGNORED_PROPERTIES.has(key)) {
      return undefined
    }
    return value
  })
}

/**
 * Map each top-level `const _cN = ...` (the constants the template compiler hoists) to its
 * normalized AST, so constants can be paired by value rather than by name.
 */
function buildConstValueMap(program: Program): Map<string, string> {
  const constMap = new Map<string, string>()
  for (const stmt of program.body as unknown as NormAstNode[]) {
    const declarations = (stmt.declarations ?? []) as NormAstNode[]
    if (stmt.type !== 'VariableDeclaration' || stmt.kind !== 'const' || declarations.length !== 1) {
      continue
    }
    const id = declarations[0].id as NormAstNode
    const name = id.name
    if (id.type !== 'Identifier' || typeof name !== 'string' || !/^_c\d+$/.test(name)) continue
    if (declarations[0].init) {
      constMap.set(name, normalizeAst(declarations[0].init))
    }
  }
  return constMap
}

/**
 * Create a mapping from one set of const names to another based on value equivalence.
 * Returns a map from source const names to target const names.
 */
function buildConstNameMapping(
  sourceConstMap: Map<string, string>,
  targetConstMap: Map<string, string>,
): Map<string, string> {
  const mapping = new Map<string, string>()
  const usedTargetNames = new Set<string>()

  // Build reverse map of target: value -> names (allow multiple constants with same value)
  const targetValueToNames = new Map<string, string[]>()
  for (const [name, value] of targetConstMap) {
    const names = targetValueToNames.get(value) || []
    names.push(name)
    targetValueToNames.set(value, names)
  }

  // All source names start as potentially "unmapped" (keeping their original name)
  // As we build mappings, track which source names will actually be renamed
  const sourceNames = new Set(sourceConstMap.keys())

  // For each source const, find matching target by value
  // Ensure each target name is used at most once to prevent duplicate declarations
  for (const [sourceName, sourceValue] of sourceConstMap) {
    const targetNames = targetValueToNames.get(sourceValue)
    if (!targetNames || targetNames.length === 0) continue

    // Try to find an unused target name, preferring the same name if available
    let targetName: string | undefined

    // First, try exact name match (e.g., source _c0 -> target _c0)
    // Only if this name isn't already used as a target
    if (targetNames.includes(sourceName) && !usedTargetNames.has(sourceName)) {
      targetName = sourceName
    } else {
      // Otherwise, find first unused target name
      // IMPORTANT: Don't use a target name that's an existing SOURCE name,
      // unless that source name will also be mapped to something else.
      // We'll handle this in a second pass.
      targetName = targetNames.find((n) => !usedTargetNames.has(n))
    }

    if (targetName) {
      mapping.set(sourceName, targetName)
      usedTargetNames.add(targetName)
    }
  }

  // Iteratively detect and fix conflicts until stable
  // A conflict occurs when:
  // 1. Source name X has no mapping (keeps its original name)
  // 2. Some other source Y maps to X (Y -> X)
  // Removing the Y -> X mapping may create new conflicts, so we iterate
  let changed = true
  while (changed) {
    changed = false

    // Find all currently unmapped source names
    const unmappedSourceNames = new Set<string>()
    for (const sourceName of sourceNames) {
      if (!mapping.has(sourceName)) {
        unmappedSourceNames.add(sourceName)
      }
    }

    // Find mappings that would create duplicates with unmapped source names
    for (const [sourceName, targetName] of mapping) {
      // If the target name is an unmapped source name (and not the same as this source),
      // this would create a duplicate declaration
      if (unmappedSourceNames.has(targetName) && sourceName !== targetName) {
        mapping.delete(sourceName)
        changed = true
        break // Restart the conflict check after modification
      }
    }
  }

  return mapping
}

/**
 * Replace const references in code using the provided mapping.
 * E.g., if mapping has _c0 -> _c1, replaces all _c0 references with _c1.
 *
 * This uses AST-based replacement for precision:
 * 1. Parse the code to get the AST
 * 2. Find all Identifier nodes whose name matches a key in constMapping
 * 3. Collect their spans (start, end positions)
 * 4. Sort spans by position descending to avoid offset shifts
 * 5. Replace each span with the mapped value from constMapping
 *
 * This is more precise than regex because it only replaces actual identifiers,
 * not substrings in strings or comments.
 */
function replaceConstReferences(code: string, constMapping: Map<string, string>): string {
  if (constMapping.size === 0) {
    return code
  }

  // Parse the code to get the AST
  const parseResult = parseSync('temp.js', code, { sourceType: 'module' })
  const ast = parseResult.program

  // Collect all identifier spans that need replacement
  const replacements: Array<{ start: number; end: number; replacement: string }> = []

  // Recursive function to traverse the AST and find identifiers
  function traverse(node: unknown): void {
    if (node === null || typeof node !== 'object') {
      return
    }

    const obj = node as Record<string, unknown>

    // Check if this is an Identifier node with a name that needs replacement
    if (obj.type === 'Identifier' && typeof obj.name === 'string') {
      const replacement = constMapping.get(obj.name)
      if (replacement !== undefined && obj.start !== undefined && obj.end !== undefined) {
        replacements.push({
          start: obj.start as number,
          end: obj.end as number,
          replacement,
        })
      }
    }

    // Recursively traverse all properties
    for (const key of Object.keys(obj)) {
      const value = obj[key]
      if (Array.isArray(value)) {
        for (const item of value) {
          traverse(item)
        }
      } else if (value !== null && typeof value === 'object') {
        traverse(value)
      }
    }
  }

  traverse(ast)

  // Sort replacements by start position descending to avoid offset issues
  replacements.sort((a, b) => b.start - a.start)

  // Apply replacements from end to start
  let result = code
  for (const { start, end, replacement } of replacements) {
    result = result.slice(0, start) + replacement + result.slice(end)
  }

  return result
}

interface Statement {
  type: string
  id?: { name: string } | null
  span?: { start: number; end: number }
  [key: string]: unknown
}

interface Program {
  body: Statement[]
  [key: string]: unknown
}

interface ExtractedFunction {
  code: string
  normalized: string
}

/**
 * Extract every named top-level function and variable declaration, exported or not.
 * Returns a map of name to its code and normalized representation.
 *
 * Handles:
 * - FunctionDeclaration: `function foo() { ... }`, `export function foo() { ... }`,
 *   `export default function foo() { ... }`
 * - VariableDeclaration: `const foo = function() { ... }`, `const _forTrack0 = (item) => item.id`,
 *   and non-function values such as the hoisted `const _c0 = ["*"]`, whose values are compared too
 */
function extractFunctions(program: Program, sourceCode: string): Map<string, ExtractedFunction> {
  const functions = new Map<string, ExtractedFunction>()

  // oxc-parser's JS API reports UTF-16 offsets, so they index the string directly.
  const add = (name: string, node: NormAstNode) => {
    const code = sourceCode.slice(node.start, node.end)
    functions.set(name, { code, normalized: normalizeAst(node) })
  }

  for (const topLevel of program.body as unknown as NormAstNode[]) {
    const statement =
      topLevel.type === 'ExportNamedDeclaration' || topLevel.type === 'ExportDefaultDeclaration'
        ? (topLevel.declaration as NormAstNode | null | undefined)
        : topLevel
    if (!statement) continue

    const id = statement.id as NormAstNode | null | undefined
    if (statement.type === 'FunctionDeclaration' && typeof id?.name === 'string') {
      add(id.name, statement)
    }

    if (statement.type === 'VariableDeclaration') {
      for (const decl of statement.declarations as NormAstNode[]) {
        const declId = decl.id as NormAstNode
        const init = decl.init as NormAstNode | null
        if (declId.type === 'Identifier' && typeof declId.name === 'string' && init) {
          add(declId.name, init)
        }
      }
    }
  }

  return functions
}

/**
 * Line diff entry for simple code comparison.
 */
interface LineDiff {
  type: 'add' | 'remove' | 'context'
  line: string
}

/**
 * Compute a line-by-line diff between two code strings using the `diff` library.
 * This preserves ordering and correctly handles duplicate lines.
 *
 * @param oxcCode - The Oxc-generated code (actual)
 * @param tsCode - The TypeScript/Angular-generated code (expected)
 * @returns Array of line differences with type and content
 */
function computeLineDiff(oxcCode: string, tsCode: string): LineDiff[] {
  const result: LineDiff[] = []

  // Use diffLines from the 'diff' library for proper LCS-based diff
  // Parameters: oldStr (expected/TS), newStr (actual/Oxc)
  const changes = diffLines(tsCode, oxcCode)

  for (const change of changes) {
    // Split change value into individual lines, filtering empty ones
    const lines = change.value.split('\n').filter((l) => l.trim())

    if (change.added) {
      // Lines in Oxc but not in TS (extra in Oxc)
      for (const line of lines) {
        result.push({ type: 'add', line })
      }
    } else if (change.removed) {
      // Lines in TS but not in Oxc (missing from Oxc)
      for (const line of lines) {
        result.push({ type: 'remove', line })
      }
    }
    // Unchanged lines (change.added === false && change.removed === false) are skipped
  }

  return result
}

/**
 * Compare top-level declarations from both outputs by name, using normalized ASTs.
 *
 * Oxc's `_cN` constants are already renamed to Angular's names (see `replaceConstReferences`),
 * so names pair up directly.
 *
 * @param oxcFunctions - Declarations extracted from Oxc output
 * @param tsFunctions - Declarations extracted from TS output
 */
function compareFunctions(
  oxcFunctions: Map<string, ExtractedFunction>,
  tsFunctions: Map<string, ExtractedFunction>,
): FunctionLevelComparison {
  const missingFunctions: string[] = []
  const extraFunctions: string[] = []
  const functionDiffs: FunctionDiff[] = []
  const matchingFunctions: string[] = []

  const matchedTsFunctions = new Set<string>()
  const matchedOxcFunctions = new Set<string>()

  for (const [name, oxcFunc] of oxcFunctions) {
    const tsFunc = tsFunctions.get(name)

    if (tsFunc) {
      matchedOxcFunctions.add(name)
      matchedTsFunctions.add(name)

      if (oxcFunc.normalized === tsFunc.normalized) {
        matchingFunctions.push(name)
      } else {
        // Compute line diff for reporting
        const lineDiffs = computeLineDiff(oxcFunc.code, tsFunc.code)
        // Convert to AstDiff format for compatibility
        const diffs: AstDiff[] = lineDiffs.map((ld, i) => ({
          path: `line[${i}]`,
          type: ld.type === 'add' ? ('extra' as const) : ('missing' as const),
          expected: ld.type === 'remove' ? ld.line : undefined,
          actual: ld.type === 'add' ? ld.line : undefined,
        }))
        // Store actual code strings for display
        functionDiffs.push({
          name: name,
          oxcCode: oxcFunc.code,
          tsCode: tsFunc.code,
          diffs,
        })
      }
    }
  }

  // Find missing functions (in TS but not matched from Oxc)
  for (const name of tsFunctions.keys()) {
    if (!matchedTsFunctions.has(name)) {
      missingFunctions.push(name)
    }
  }

  // Find extra functions (in Oxc but not matched to TS)
  for (const name of oxcFunctions.keys()) {
    if (!matchedOxcFunctions.has(name)) {
      extraFunctions.push(name)
    }
  }

  // Sort for consistent output
  missingFunctions.sort()
  extraFunctions.sort()
  matchingFunctions.sort()
  functionDiffs.sort((a, b) => a.name.localeCompare(b.name))

  return {
    missingFunctions,
    extraFunctions,
    functionDiffs,
    matchingFunctions,
  }
}

/**
 * Format function-level comparison for display.
 */
export function formatFunctionComparison(comparison: FunctionLevelComparison): string {
  const lines: string[] = []

  if (comparison.missingFunctions.length > 0) {
    lines.push('Missing functions (in TypeScript but not in Oxc):')
    for (const name of comparison.missingFunctions) {
      lines.push(`  - ${name}`)
    }
    lines.push('')
  }

  if (comparison.extraFunctions.length > 0) {
    lines.push('Extra functions (in Oxc but not in TypeScript):')
    for (const name of comparison.extraFunctions) {
      lines.push(`  + ${name}`)
    }
    lines.push('')
  }

  if (comparison.functionDiffs.length > 0) {
    lines.push('Functions with differences:')
    for (const { name, diffs } of comparison.functionDiffs) {
      lines.push(`  ~ ${name} (${diffs.length} line diff${diffs.length === 1 ? '' : 's'}):`)
      // Show first few diffs for each function
      const maxDiffs = 5
      for (let i = 0; i < Math.min(diffs.length, maxDiffs); i++) {
        const d = diffs[i]
        lines.push(`      ${formatSingleDiff(d)}`)
      }
      if (diffs.length > maxDiffs) {
        lines.push(`      ... and ${diffs.length - maxDiffs} more`)
      }
    }
    lines.push('')
  }

  if (comparison.matchingFunctions.length > 0) {
    lines.push(`Matching functions: ${comparison.matchingFunctions.length}`)
  }

  return lines.join('\n')
}

/**
 * Format a single diff entry.
 */
function formatSingleDiff(d: AstDiff): string {
  switch (d.type) {
    case 'missing':
      return `- ${d.expected}`
    case 'extra':
      return `+ ${d.actual}`
    case 'different':
      return `~ expected: ${stringify(d.expected)} | actual: ${stringify(d.actual)}`
  }
}

/**
 * Format AST diffs for display (legacy).
 */
export function formatDiffs(diffs: AstDiff[]): string {
  return diffs
    .map((d) => {
      switch (d.type) {
        case 'missing':
          return `- ${d.path}: missing (expected: ${stringify(d.expected)})`
        case 'extra':
          return `+ ${d.path}: extra (actual: ${stringify(d.actual)})`
        case 'different':
          return `~ ${d.path}: different\n    expected: ${stringify(d.expected)}\n    actual: ${stringify(d.actual)}`
      }
    })
    .join('\n')
}

function stringify(value: unknown): string {
  if (typeof value === 'string') return value
  if (typeof value === 'object') {
    try {
      const str = JSON.stringify(value)
      return str.length > 100 ? str.slice(0, 100) + '...' : str
    } catch {
      return '[object]'
    }
  }
  // oxlint-disable-next-line no-base-to-string
  return String(value)
}

// ============================================================================
// Full-file semantic comparison
// ============================================================================

/**
 * Normalized import information for comparison.
 */
interface NormalizedImport {
  /** Module source (e.g., "@angular/core") */
  moduleSource: string
  /** Sorted list of imported specifiers (local names) */
  specifiers: string[]
  /** Whether this is a namespace import (import * as x) */
  isNamespace: boolean
  /** Namespace name if isNamespace is true */
  namespaceName?: string
}

/**
 * Normalized export information for comparison.
 */
interface NormalizedExport {
  /** Exported name */
  exportName: string
  /** Local name (for re-exports) */
  localName?: string
  /** Module source (for re-exports) */
  moduleSource?: string
  /** Whether this is a type export */
  isType: boolean
}

/**
 * Static field assignment extracted from code.
 */
interface StaticFieldAssignment {
  /** Class name */
  className: string
  /** Field name (e.g., "ɵcmp", "ɵfac") */
  fieldName: string
  /** Assignment value source text (for reporting) */
  value: string
  /** Assignment value AST (for comparison) */
  node: NormAstNode
}

/**
 * Extract normalized imports from parse result's module.
 */
function extractNormalizedImports(staticImports: StaticImport[]): NormalizedImport[] {
  const imports: NormalizedImport[] = []

  for (const imp of staticImports) {
    const moduleSource = imp.moduleRequest.value
    const specifiers: string[] = []
    let isNamespace = false
    let namespaceName: string | undefined

    for (const entry of imp.entries) {
      // Skip type-only imports
      if (entry.isType) continue

      if (entry.importName.kind === 'NamespaceObject') {
        isNamespace = true
        namespaceName = entry.localName.value
      } else if (entry.importName.kind === 'Default') {
        specifiers.push(`default as ${entry.localName.value}`)
      } else if (entry.importName.kind === 'Name') {
        const importedName = entry.importName.name
        const localName = entry.localName.value
        if (importedName === localName) {
          specifiers.push(localName)
        } else {
          specifiers.push(`${importedName} as ${localName}`)
        }
      }
    }

    // Sort specifiers for consistent comparison
    specifiers.sort()

    imports.push({
      moduleSource,
      specifiers,
      isNamespace,
      namespaceName,
    })
  }

  return imports
}

/**
 * Extract normalized exports from parse result's module.
 */
function extractNormalizedExports(staticExports: StaticExport[]): NormalizedExport[] {
  const exports: NormalizedExport[] = []

  for (const exp of staticExports) {
    for (const entry of exp.entries) {
      // Skip type-only exports
      if (entry.isType) continue

      const exportName = entry.exportName.name || ''
      const localName = entry.localName.name || undefined
      const moduleSource = entry.moduleRequest?.value || undefined

      exports.push({
        exportName,
        localName,
        moduleSource,
        isType: entry.isType,
      })
    }
  }

  return exports
}

/**
 * Compare imports between Oxc and TS outputs.
 * Returns differences found.
 */
function compareImports(
  oxcImports: NormalizedImport[],
  tsImports: NormalizedImport[],
): ImportDiff[] {
  const diffs: ImportDiff[] = []

  // Build maps by module source
  const oxcByModule = new Map<string, NormalizedImport>()
  const tsByModule = new Map<string, NormalizedImport>()

  for (const imp of oxcImports) {
    // Merge imports from the same module
    const existing = oxcByModule.get(imp.moduleSource)
    if (existing) {
      existing.specifiers = [...new Set([...existing.specifiers, ...imp.specifiers])].sort()
      if (imp.isNamespace) {
        existing.isNamespace = true
        existing.namespaceName = imp.namespaceName
      }
    } else {
      oxcByModule.set(imp.moduleSource, { ...imp })
    }
  }

  for (const imp of tsImports) {
    // Merge imports from the same module
    const existing = tsByModule.get(imp.moduleSource)
    if (existing) {
      existing.specifiers = [...new Set([...existing.specifiers, ...imp.specifiers])].sort()
      if (imp.isNamespace) {
        existing.isNamespace = true
        existing.namespaceName = imp.namespaceName
      }
    } else {
      tsByModule.set(imp.moduleSource, { ...imp })
    }
  }

  // Find missing imports (in TS but not in Oxc)
  for (const [moduleSource, tsImp] of tsByModule) {
    const oxcImp = oxcByModule.get(moduleSource)
    if (!oxcImp) {
      diffs.push({
        type: 'missing',
        moduleSource,
        expected: tsImp.isNamespace ? [`* as ${tsImp.namespaceName}`] : tsImp.specifiers,
      })
    } else {
      // Check for specifier differences
      const tsSpecs = new Set(tsImp.specifiers)
      const oxcSpecs = new Set(oxcImp.specifiers)

      const missingSpecs = [...tsSpecs].filter((s) => !oxcSpecs.has(s))
      const extraSpecs = [...oxcSpecs].filter((s) => !tsSpecs.has(s))

      if (missingSpecs.length > 0 || extraSpecs.length > 0) {
        diffs.push({
          type: 'different',
          moduleSource,
          expected: tsImp.specifiers,
          actual: oxcImp.specifiers,
        })
      }
    }
  }

  // Find extra imports (in Oxc but not in TS)
  for (const [moduleSource, oxcImp] of oxcByModule) {
    if (!tsByModule.has(moduleSource)) {
      diffs.push({
        type: 'extra',
        moduleSource,
        actual: oxcImp.isNamespace ? [`* as ${oxcImp.namespaceName}`] : oxcImp.specifiers,
      })
    }
  }

  return diffs
}

/**
 * Compare exports between Oxc and TS outputs.
 * Returns differences found.
 */
function compareExports(
  oxcExports: NormalizedExport[],
  tsExports: NormalizedExport[],
): ExportDiff[] {
  const diffs: ExportDiff[] = []

  // Build maps by export name
  const oxcByName = new Map<string, NormalizedExport>()
  const tsByName = new Map<string, NormalizedExport>()

  for (const exp of oxcExports) {
    if (exp.exportName) {
      oxcByName.set(exp.exportName, exp)
    }
  }

  for (const exp of tsExports) {
    if (exp.exportName) {
      tsByName.set(exp.exportName, exp)
    }
  }

  // Find missing exports (in TS but not in Oxc)
  for (const [name, tsExp] of tsByName) {
    if (!oxcByName.has(name)) {
      diffs.push({
        type: 'missing',
        exportName: name,
        expected: tsExp.localName || name,
      })
    }
  }

  // Find extra exports (in Oxc but not in TS)
  for (const [name, oxcExp] of oxcByName) {
    if (!tsByName.has(name)) {
      diffs.push({
        type: 'extra',
        exportName: name,
        actual: oxcExp.localName || name,
      })
    }
  }

  return diffs
}

/** Whether a class member is a `static ɵxxx = ...` field, which is compared on its own. */
function isAngularStaticField(member: NormAstNode): boolean {
  const key = member.key as NormAstNode | undefined
  return (
    member.type === 'PropertyDefinition' &&
    member.static === true &&
    key?.type === 'Identifier' &&
    typeof key.name === 'string' &&
    key.name.startsWith('ɵ')
  )
}

/**
 * Extract Angular static fields, both the `static ɵxxx = ...` class fields current Angular and
 * Oxc emit (e.g. `static ɵcmp = i0.ɵɵdefineComponent(...)`) and top-level
 * `ClassName.ɵxxx = ...` assignments.
 */
function extractClassStaticFields(program: Program, sourceCode: string): StaticFieldAssignment[] {
  const fields: StaticFieldAssignment[] = []
  // oxc-parser's JS API reports UTF-16 offsets, so they index the string directly.
  const push = (className: string, fieldName: string, node: NormAstNode) => {
    const value = sourceCode.slice(node.start, node.end).trim()
    fields.push({ className, fieldName, value, node })
  }
  for (const statement of program.body as unknown as NormAstNode[]) {
    if (statement.type === 'ExpressionStatement') {
      const assignment = statement.expression as NormAstNode
      const target = assignment.left as NormAstNode | undefined
      const object = target?.object as NormAstNode | undefined
      const property = target?.property as NormAstNode | undefined
      if (
        assignment.type === 'AssignmentExpression' &&
        assignment.operator === '=' &&
        target?.type === 'MemberExpression' &&
        target.computed !== true &&
        object?.type === 'Identifier' &&
        typeof object.name === 'string' &&
        typeof property?.name === 'string' &&
        property.name.startsWith('ɵ')
      ) {
        push(object.name, property.name, assignment.right as NormAstNode)
      }
      continue
    }
    const exported =
      statement.type === 'ExportNamedDeclaration' || statement.type === 'ExportDefaultDeclaration'
    const cls = (exported ? statement.declaration : statement) as NormAstNode | undefined
    const className = (cls?.id as NormAstNode | undefined)?.name
    if (cls?.type !== 'ClassDeclaration' || typeof className !== 'string') continue
    for (const member of (cls.body as NormAstNode).body as NormAstNode[]) {
      if (isAngularStaticField(member) && member.value) {
        push(className, (member.key as NormAstNode).name as string, member.value as NormAstNode)
      }
    }
  }
  return fields
}

// ============================================================================
// Class metadata (setClassMetadata) extraction and comparison
// ============================================================================

/**
 * Extracted class metadata from setClassMetadata call.
 *
 * The setClassMetadata call looks like:
 * ```javascript
 * i0.ɵɵsetClassMetadata(ClassName,
 *   [{type: Component, args: [{...}]}],  // decorators
 *   null,  // ctorParams
 *   null   // propDecorators
 * );
 * ```
 */
interface ClassMetadataInfo {
  /** Class name (first argument) */
  className: string
  /** Decorators array (second argument) */
  decorators: string
  /** Constructor parameters (third argument, null if none) */
  ctorParams: string | null
  /** Property decorators (fourth argument, null if none) */
  propDecorators: string | null
  /** `ɵsetClassMetadataAsync` call: resolver thunk (second argument). */
  resolver?: string | null
  /** `ɵsetClassMetadataAsync` call: wrapper with params alpha-renamed, normalized AST. */
  wrapperNormalized?: string | null
  /** Whether this entry came from `ɵsetClassMetadataAsync` (deferred imports). */
  isAsync?: boolean
}

/**
 * Extract setClassMetadata calls from compiled code.
 *
 * Parses patterns like:
 * ```javascript
 * i0.ɵɵsetClassMetadata(ClassName, [...], null, null);
 * ```
 *
 * @param code - The compiled JavaScript code
 * @returns Array of extracted class metadata info
 */

/**
 * `null` and `undefined` are NOT interchangeable in these slots: the runtime
 * `setClassMetadata` guards each field with `!== null`, so `undefined` (or a
 * missing argument, which is `undefined` at the call site) assigns the field
 * while `null` skips it. Keep the two spellings distinct so the comparator
 * catches a regression between them. Returns the literal argument text, or
 * `'null'` / `'undefined'` for the absent-value forms.
 */
function metadataArgOrAbsent(arg: string | undefined): string {
  if (arg === undefined) return 'undefined'
  const trimmed = arg.trim()
  if (trimmed === 'null') return 'null'
  // `undefined`, `void 0` and `void(0)` are the inert spellings a compiler
  // emits; any other `void expr` runs its operand and must stay distinct.
  if (trimmed === 'undefined' || trimmed === 'void 0' || trimmed === 'void(0)') return 'undefined'
  return trimmed
}

/**
 * Namespace bindings (`import * as X` / `const X = require(...)`) for
 * '@angular/core' in the emitted code — the callee of a metadata call must be
 * one of these so a user-defined `debug.ɵsetClassMetadata` isn't picked up.
 */
function angularCoreNamespaces(program: unknown): Set<string> {
  const namespaces = new Set<string>()
  walkAst(program, (node) => {
    if (node.type === 'ImportDeclaration') {
      const source = node.source as NormAstNode | undefined
      if (source?.value !== '@angular/core') return
      for (const spec of (node.specifiers ?? []) as NormAstNode[]) {
        if (spec.type === 'ImportNamespaceSpecifier') {
          namespaces.add((spec.local as NormAstNode).name as string)
        }
      }
    } else if (node.type === 'VariableDeclaration') {
      // `const i0 = require('@angular/core')`
      for (const decl of (node.declarations ?? []) as NormAstNode[]) {
        const init = decl.init as NormAstNode | undefined
        const callee = init?.callee as NormAstNode | undefined
        if (
          init?.type === 'CallExpression' &&
          callee?.type === 'Identifier' &&
          callee.name === 'require' &&
          (init.arguments as NormAstNode[])?.[0]?.value === '@angular/core'
        ) {
          const id = decl.id as NormAstNode
          if (id?.type === 'Identifier') namespaces.add(id.name as string)
        }
      }
    }
  })
  return namespaces
}

/**
 * Alpha-rename an arrow/function's parameters to `p0`, `p1`, ... so equivalent
 * wrappers compare equal while a swapped parameter usage order still differs.
 *
 * Only identifier *references* are renamed. Identifier nodes in positions that
 * are not variable references — non-computed member properties (`registry.X`),
 * non-computed object keys (`{X: ...}`, including the key of a shorthand
 * `{X}`), class member keys, labels, and `import.meta` — keep their names so
 * observable names stay in the normalized form.
 */
function normalizeWrapperAst(wrapper: NormAstNode): string {
  const cloned = JSON.parse(JSON.stringify(wrapper)) as NormAstNode
  const paramNames: string[] = []
  for (const param of (cloned.params ?? []) as NormAstNode[]) {
    if (param.type === 'Identifier') paramNames.push(param.name as string)
  }
  const nonReference = new Set<NormAstNode>()
  const protect = (node: unknown) => {
    const ident = node as NormAstNode | undefined
    if (ident?.type === 'Identifier') nonReference.add(ident)
  }
  walkAst(cloned, (node) => {
    if (
      (node.type === 'MemberExpression' || node.type === 'OptionalMemberExpression') &&
      node.computed !== true
    ) {
      protect(node.property)
    } else if (
      (node.type === 'Property' ||
        node.type === 'PropertyDefinition' ||
        node.type === 'MethodDefinition' ||
        node.type === 'AccessorProperty') &&
      node.computed !== true
    ) {
      protect(node.key)
    } else if (node.type === 'LabeledStatement') {
      protect(node.label)
    } else if (node.type === 'BreakStatement' || node.type === 'ContinueStatement') {
      protect(node.label)
    } else if (node.type === 'MetaProperty') {
      protect(node.meta)
      protect(node.property)
    }
  })
  // Rename references bound to the wrapper's parameters before the parameter
  // declarations: scope tracking must see the ORIGINAL names, and renaming
  // declarations first would let a canonical name (`#p1`) collide with an
  // original parameter and corrupt the shadow set. The canonical spellings
  // `#p0`, `#p1`, ... can never be real identifiers, so a free `p1` in the
  // body cannot collide with a renamed parameter either.
  const canonical = (index: number) => `#p${index}`
  const active = new Map(paramNames.map((name, index) => [name, canonical(index)]))
  if (cloned.body) renameBoundIdentifiers(cloned.body, active, nonReference)
  for (const [index, param] of ((cloned.params ?? []) as NormAstNode[]).entries()) {
    if (param.type === 'Identifier') param.name = canonical(index)
  }
  return normalizeAst(cloned)
}

/**
 * Collect names bound by a binding pattern — parameter lists, destructuring
 * targets, variable declarator ids and catch params. Expression parts
 * (default values, computed keys) are not bindings and are skipped.
 */
function boundNames(pattern: NormAstNode | null | undefined, out: Set<string>): void {
  if (!pattern || typeof pattern !== 'object') return
  switch (pattern.type) {
    case 'Identifier':
      out.add(pattern.name as string)
      break
    case 'ObjectPattern':
      for (const prop of (pattern.properties ?? []) as NormAstNode[]) {
        if (prop.type === 'Property') boundNames(prop.value as NormAstNode, out)
        else if (prop.type === 'RestElement') boundNames(prop.argument as NormAstNode, out)
      }
      break
    case 'ArrayPattern':
      for (const el of (pattern.elements ?? []) as (NormAstNode | null)[]) boundNames(el, out)
      break
    case 'AssignmentPattern':
      boundNames(pattern.left as NormAstNode, out)
      break
    case 'RestElement':
      boundNames(pattern.argument as NormAstNode, out)
      break
    case 'TSParameterProperty':
      boundNames(pattern.parameter as NormAstNode, out)
      break
  }
}

/**
 * Names bound directly inside a block/program/statement list by `let`,
 * `const`, `class` and `function` declarations. Only direct members are
 * scanned — nested blocks manage their own scope.
 */
function blockDeclNames(stmts: NormAstNode[] | undefined, out: Set<string>): void {
  for (const stmt of stmts ?? []) {
    if (stmt.type === 'VariableDeclaration' && stmt.kind !== 'var') {
      for (const decl of (stmt.declarations ?? []) as NormAstNode[]) {
        boundNames(decl.id as NormAstNode, out)
      }
    } else if (stmt.type === 'FunctionDeclaration' || stmt.type === 'ClassDeclaration') {
      const id = stmt.id as NormAstNode | undefined
      if (id?.type === 'Identifier') out.add(id.name as string)
    }
  }
}

/**
 * `var`-bound names anywhere inside a function body — they hoist to the
 * function scope. Nested functions, classes, and class static blocks are not
 * descended into: a `var` in a static block scopes to the block itself, and
 * class members can't contain `var` statements.
 */
function varDeclNames(node: unknown, out: Set<string>): void {
  if (node === null || typeof node !== 'object') return
  if (Array.isArray(node)) {
    for (const item of node) varDeclNames(item, out)
    return
  }
  const obj = node as NormAstNode
  if (
    obj.type === 'FunctionDeclaration' ||
    obj.type === 'FunctionExpression' ||
    obj.type === 'ArrowFunctionExpression' ||
    obj.type === 'StaticBlock' ||
    obj.type === 'ClassDeclaration' ||
    obj.type === 'ClassExpression'
  ) {
    return
  }
  if (obj.type === 'VariableDeclaration' && obj.kind === 'var') {
    for (const decl of (obj.declarations ?? []) as NormAstNode[]) {
      boundNames(decl.id as NormAstNode, out)
    }
    return
  }
  for (const value of Object.values(obj)) {
    if (value !== null && typeof value === 'object') varDeclNames(value, out)
  }
}

/**
 * Rename identifiers bound to names in `active` (param name → `p<i>`),
 * respecting lexical scope: parameters, `let`/`const`/`class`/`function`
 * declarations, hoisted `var`s, catch params and class names that re-declare
 * an active name shadow it for their subtree, and identifiers in
 * `nonReference` positions are never renamed.
 */
function renameBoundIdentifiers(
  node: unknown,
  active: ReadonlyMap<string, string>,
  nonReference: ReadonlySet<NormAstNode>,
): void {
  if (node === null || typeof node !== 'object') return
  if (Array.isArray(node)) {
    for (const item of node) renameBoundIdentifiers(item, active, nonReference)
    return
  }
  const obj = node as NormAstNode
  if (obj.type === 'Identifier') {
    if (!nonReference.has(obj)) {
      const mapped = active.get(obj.name as string)
      if (mapped) obj.name = mapped
    }
    return
  }

  // Narrow the active renames for this node's subtree when it introduces
  // shadowing bindings.
  let inner: Map<string, string> = new Map(active)
  const type = obj.type
  if (
    type === 'FunctionDeclaration' ||
    type === 'FunctionExpression' ||
    type === 'ArrowFunctionExpression'
  ) {
    // Parameter initializers evaluate in the parameter scope — only the
    // function's own params/id shadow there. Body bindings (vars and the
    // body's top-level let/const/class/function decls) apply to the body
    // only; otherwise a `let A` in the body would hide the outer mapping
    // for a default `(x = A)`.
    const paramInner = new Map(active)
    for (const param of (obj.params ?? []) as NormAstNode[]) {
      const names = new Set<string>()
      boundNames(param, names)
      for (const name of names) paramInner.delete(name)
    }
    const id = obj.id as NormAstNode | undefined
    if (id?.type === 'Identifier') paramInner.delete(id.name as string)

    renameBoundIdentifiers(obj.params, paramInner, nonReference)

    const bodyShadowed = new Set<string>()
    varDeclNames(obj.body, bodyShadowed)
    const body = obj.body as NormAstNode | undefined
    if (body?.type === 'BlockStatement') {
      blockDeclNames(body.body as NormAstNode[], bodyShadowed)
    }
    const bodyInner = new Map(paramInner)
    for (const name of bodyShadowed) bodyInner.delete(name)
    renameBoundIdentifiers(obj.body, bodyInner, nonReference)
    return
  } else if (type === 'SwitchStatement') {
    // The discriminant is evaluated in the enclosing scope, before the
    // case-block bindings exist — only the cases see the shadowed names.
    const shadowed = new Set<string>()
    for (const c of (obj.cases ?? []) as NormAstNode[]) {
      blockDeclNames(c.consequent as NormAstNode[], shadowed)
    }
    renameBoundIdentifiers(obj.discriminant, active, nonReference)
    if (shadowed.size > 0) {
      inner = new Map(active)
      for (const name of shadowed) inner.delete(name)
    }
    renameBoundIdentifiers(obj.cases, inner, nonReference)
    return
  } else if (type === 'StaticBlock') {
    // A static block is its own `var` scope — collect both its hoisted vars
    // and its top-level lexical declarations.
    const shadowed = new Set<string>()
    varDeclNames(obj.body, shadowed)
    blockDeclNames(obj.body as NormAstNode[], shadowed)
    if (shadowed.size > 0) {
      inner = new Map(active)
      for (const name of shadowed) inner.delete(name)
    }
  } else if (type === 'Program' || type === 'BlockStatement') {
    const shadowed = new Set<string>()
    blockDeclNames(obj.body as NormAstNode[], shadowed)
    if (shadowed.size > 0) {
      inner = new Map(active)
      for (const name of shadowed) inner.delete(name)
    }
  } else if (type === 'CatchClause') {
    const param = obj.param as NormAstNode | undefined
    if (param) {
      const shadowed = new Set<string>()
      boundNames(param, shadowed)
      inner = new Map(active)
      for (const name of shadowed) inner.delete(name)
    }
  } else if (type === 'ClassExpression' || type === 'ClassDeclaration') {
    const id = obj.id as NormAstNode | undefined
    if (id?.type === 'Identifier' && active.has(id.name as string)) {
      inner = new Map(active)
      inner.delete(id.name as string)
    }
  } else if (type === 'ForStatement' || type === 'ForInStatement' || type === 'ForOfStatement') {
    const decl = (type === 'ForStatement' ? obj.init : obj.left) as NormAstNode | undefined
    if (decl?.type === 'VariableDeclaration' && decl.kind !== 'var') {
      const shadowed = new Set<string>()
      for (const d of (decl.declarations ?? []) as NormAstNode[]) {
        boundNames(d.id as NormAstNode, shadowed)
      }
      inner = new Map(active)
      for (const name of shadowed) inner.delete(name)
    }
  }

  for (const value of Object.values(obj)) {
    if (value !== null && typeof value === 'object') {
      renameBoundIdentifiers(value, inner, nonReference)
    }
  }
}

/**
 * Extract ɵsetClassMetadata/ɵsetClassMetadataAsync calls from the parsed AST.
 * Working on the tree avoids every text-scanning edge case: regex literals,
 * comments, nested calls and `$`-or-unicode identifiers are all handled by
 * the parser.
 */
export function extractClassMetadataCalls(program: unknown, code: string): ClassMetadataInfo[] {
  const results: ClassMetadataInfo[] = []
  const coreNamespaces = angularCoreNamespaces(program)
  // `ɵsetClassMetadata` calls nested inside an async wrapper are already
  // covered by `wrapperNormalized` — extracting them again would compare the
  // raw parameter spellings and produce spurious diffs (`A` vs `B`).
  const callsInsideAsyncWrappers = new Set<NormAstNode>()

  const slice = (node: NormAstNode | undefined): string | undefined =>
    node && typeof node.start === 'number' && typeof node.end === 'number'
      ? code.slice(node.start, node.end)
      : undefined

  walkAst(program, (node) => {
    if (node.type !== 'CallExpression') return
    const callee = node.callee as NormAstNode | undefined
    if (!callee || callee.type !== 'MemberExpression') return
    const object = callee.object as NormAstNode | undefined
    const property = callee.property as NormAstNode | undefined
    if (object?.type !== 'Identifier' || property?.type !== 'Identifier') return
    if (!coreNamespaces.has(object.name as string)) return

    const isAsync = property.name === 'ɵsetClassMetadataAsync'
    if (!isAsync && property.name !== 'ɵsetClassMetadata') return

    const args = node.arguments as NormAstNode[] | undefined
    const classArg = args?.[0]
    if (!args || !classArg) return

    if (callsInsideAsyncWrappers.has(node)) return

    const className =
      classArg.type === 'Identifier' ? (classArg.name as string) : (slice(classArg) ?? '')

    if (isAsync) {
      // `ɵsetClassMetadataAsync(Class, resolver, wrapper)` — the wrapper's
      // nested calls are normalized with the parameter bindings, so mark every
      // nested metadata call in the wrapper argument as covered.
      const wrapper = args[2]
      if (wrapper) {
        walkAst(wrapper, (nested) => {
          if (nested !== wrapper && nested.type === 'CallExpression') {
            callsInsideAsyncWrappers.add(nested)
          }
        })
      }
      results.push({
        className,
        decorators: 'null',
        ctorParams: null,
        propDecorators: null,
        resolver: metadataArgOrAbsent(slice(args[1])),
        isAsync: true,
        wrapperNormalized:
          wrapper &&
          (wrapper.type === 'ArrowFunctionExpression' || wrapper.type === 'FunctionExpression')
            ? normalizeWrapperAst(wrapper)
            : (slice(wrapper) ?? null),
      })
      return
    }

    results.push({
      className,
      decorators: metadataArgOrAbsent(slice(args[1])),
      ctorParams: metadataArgOrAbsent(slice(args[2])),
      propDecorators: metadataArgOrAbsent(slice(args[3])),
    })
  })

  return results
}

/**
 * Normalize a metadata argument for comparison.
 *
 * The argument is an expression fragment (`[...]`, `{...}`, `null`, ...), so
 * parse it and compare normalized ASTs: quote style, indentation, trailing
 * commas and key ordering noise don't matter semantically for these
 * structures. A fragment that can't be parsed is compared as exact text.
 */
function normalizeMetadataString(s: string | null): string {
  if (!s) return 'null'
  const parsed = parseSync('metadata.js', `(${s}\n)`, { sourceType: 'module' })
  const stmt = parsed.program.body[0]
  if (parsed.errors.length > 0 || !stmt || stmt.type !== 'ExpressionStatement') {
    return `unparsed:${s}`
  }
  return normalizeAst(stmt.expression)
}

/**
 * Compare class metadata between Oxc and TS outputs.
 *
 * @param oxcMetadata - Class metadata extracted from Oxc output
 * @param tsMetadata - Class metadata extracted from TS output
 * @returns Array of differences found
 */
export function compareClassMetadata(
  oxcMetadata: ClassMetadataInfo[],
  tsMetadata: ClassMetadataInfo[],
): ClassMetadataDiff[] {
  const diffs: ClassMetadataDiff[] = []
  // Sync and async calls for the same class are separate entries — a class
  // reported under both names on one side and only sync on the other is a
  // real difference (deferredImports metadata wrapper). Each key maps to a
  // LIST of calls: a source file can legitimately contain its own
  // `ng.ɵsetClassMetadata(...)` call through the same Angular namespace
  // binding, and keeping only the last call would let an identical user call
  // mask a divergent generated one.
  const keyOf = (m: ClassMetadataInfo) => (m.isAsync ? `${m.className}#async` : m.className)
  const groupByKey = (list: ClassMetadataInfo[]) => {
    const map = new Map<string, ClassMetadataInfo[]>()
    for (const m of list) {
      const key = keyOf(m)
      const group = map.get(key)
      if (group) {
        group.push(m)
      } else {
        map.set(key, [m])
      }
    }
    // Calls stay in execution order: repeated setClassMetadata calls overwrite
    // fields, so the same calls in a different order produce different runtime
    // metadata and must not compare equal.
    return map
  }
  const tsMap = groupByKey(tsMetadata)
  const oxcMap = groupByKey(oxcMetadata)

  // Check for missing/different metadata (in TS but not matching in Oxc)
  for (const [key, tsGroup] of tsMap) {
    const className = tsGroup[0].className
    const callName = tsGroup[0].isAsync ? 'setClassMetadataAsync' : 'setClassMetadata'
    const oxcGroup = oxcMap.get(key)
    if (!oxcGroup) {
      diffs.push({
        type: 'missing',
        className,
        field: callName,
        expected: `decorators: ${tsGroup[0].decorators.slice(0, 100)}...`,
      })
      continue
    }
    if (oxcGroup.length !== tsGroup.length) {
      diffs.push({
        type: 'different',
        className,
        field: callName,
        expected: `${tsGroup.length} call(s)`,
        actual: `${oxcGroup.length} call(s)`,
      })
    }
    for (let i = 0; i < Math.min(tsGroup.length, oxcGroup.length); i++) {
      const tsInfo = tsGroup[i]
      const oxcInfo = oxcGroup[i]
      if (tsInfo.isAsync) {
        // Async entries: compare the deferred-imports resolver thunk AND the
        // wrapper (alpha-renamed) — a swapped `(B, A)` wrapper is behaviorally
        // wrong even though its nested sync call looks identical. The nested
        // sync call itself is extracted as its own entry.
        if (
          normalizeMetadataString(tsInfo.resolver ?? null) !==
          normalizeMetadataString(oxcInfo.resolver ?? null)
        ) {
          diffs.push({
            type: 'different',
            className,
            field: 'setClassMetadataAsync.resolver',
            expected: String(tsInfo.resolver).slice(0, 100),
            actual: String(oxcInfo.resolver).slice(0, 100),
          })
        }
        if ((tsInfo.wrapperNormalized ?? null) !== (oxcInfo.wrapperNormalized ?? null)) {
          diffs.push({
            type: 'different',
            className,
            field: 'setClassMetadataAsync.wrapper',
            expected: '(normalized wrapper)',
            actual: '(normalized wrapper)',
          })
        }
      } else {
        // Compare decorators (normalized)
        const normalizedTsDecorators = normalizeMetadataString(tsInfo.decorators)
        const normalizedOxcDecorators = normalizeMetadataString(oxcInfo.decorators)
        if (normalizedTsDecorators !== normalizedOxcDecorators) {
          diffs.push({
            type: 'different',
            className,
            field: 'setClassMetadata.decorators',
            expected: tsInfo.decorators.slice(0, 100),
            actual: oxcInfo.decorators.slice(0, 100),
          })
        }

        // Compare ctorParams
        if (
          normalizeMetadataString(tsInfo.ctorParams) !== normalizeMetadataString(oxcInfo.ctorParams)
        ) {
          diffs.push({
            type: 'different',
            className,
            field: 'setClassMetadata.ctorParams',
            expected: String(tsInfo.ctorParams).slice(0, 100),
            actual: String(oxcInfo.ctorParams).slice(0, 100),
          })
        }

        // Compare propDecorators
        if (
          normalizeMetadataString(tsInfo.propDecorators) !==
          normalizeMetadataString(oxcInfo.propDecorators)
        ) {
          diffs.push({
            type: 'different',
            className,
            field: 'setClassMetadata.propDecorators',
            expected: String(tsInfo.propDecorators).slice(0, 100),
            actual: String(oxcInfo.propDecorators).slice(0, 100),
          })
        }
      }
    }
  }

  // Check for extra metadata (in Oxc but not in TS)
  for (const [key, oxcGroup] of oxcMap) {
    if (!tsMap.has(key)) {
      diffs.push({
        type: 'extra',
        className: oxcGroup[0].className,
        field: oxcGroup[0].isAsync ? 'setClassMetadataAsync' : 'setClassMetadata',
        actual: 'found in Oxc output but not in TS',
      })
    }
  }

  // Call order is observable across classes too: decorator arguments may run
  // evaluated expressions, so emitting the same calls in a different sequence
  // can change runtime behavior. When both outputs contain the same multiset
  // of calls (keyed by class/kind) but in a different order, report it at the
  // first divergent position. Same-multiset-only keeps the check meaningful:
  // differing lengths are already covered by missing/extra diffs above.
  const tsKeys = tsMetadata.map(keyOf)
  const oxcKeys = oxcMetadata.map(keyOf)
  const tsSorted = [...tsKeys].sort()
  const oxcSorted = [...oxcKeys].sort()
  if (tsSorted.length === oxcSorted.length && tsSorted.every((k, i) => k === oxcSorted[i])) {
    const divergent = tsKeys.findIndex((k, i) => k !== oxcKeys[i])
    if (divergent >= 0) {
      const callName = tsKeys[divergent].endsWith('#async')
        ? 'setClassMetadataAsync'
        : 'setClassMetadata'
      diffs.push({
        type: 'different',
        className: tsKeys[divergent].replace(/#async$/, ''),
        field: `${callName}.order`,
        expected: `call #${divergent + 1}: ${tsKeys[divergent].replace(/#async$/, '')}`,
        actual: `call #${divergent + 1}: ${oxcKeys[divergent].replace(/#async$/, '')}`,
      })
    }
  }

  return diffs
}

/**
 * Compare two static field values (e.g. the `i0.ɵɵdefineComponent({...})` of `ɵcmp`) by AST,
 * ignoring differences that do not change behaviour:
 *
 * - formatting, trailing commas and string escapes (`"\\uFFFD"` vs `"�"`)
 * - `consts: () => {...}` vs `consts: function () {...}`
 * - `$localize\`...\`` vs `$localize(__makeTemplateObject(cooked, raw), ...)`, comparing the
 *   cooked and raw strings and the substitutions
 * - `pure: true` in `ɵɵdefinePipe`, which is the default
 * - Closure-only i18n metadata that Oxc does not emit: the `goog.getMsg` variable names
 *   (`MSG_EXTERNAL_<id>$$<file>` in Angular) and the `original_code` argument of `goog.getMsg`
 *
 * String contents, including whitespace inside them, must match exactly.
 */
function staticFieldValuesEquivalent(oxcValue: NormAstNode, tsValue: NormAstNode): boolean {
  return canonicalStaticFieldValue(oxcValue) === canonicalStaticFieldValue(tsValue)
}

function canonicalStaticFieldValue(value: NormAstNode): string {
  return normalizeAst(canonicalizeNode(value, new Map()))
}

/** The string values of an array of string literals, or null if it is not one. */
function stringElements(node: unknown): string[] | null {
  const array = node as NormAstNode
  if (array?.type !== 'ArrayExpression') return null
  const strings = (array.elements as NormAstNode[]).map((element) =>
    element?.type === 'Literal' && typeof element.value === 'string' ? element.value : null,
  )
  return strings.every((value) => value !== null) ? (strings as string[]) : null
}

function canonicalizeNode(node: unknown, closureNames: Map<string, string>): unknown {
  if (Array.isArray(node)) {
    return node.map((element) => canonicalizeNode(element, closureNames))
  }
  if (node === null || typeof node !== 'object') {
    return node
  }
  const ast = node as NormAstNode
  const child = (value: unknown) => value as NormAstNode | undefined
  const children = (value: unknown) => (value ?? []) as NormAstNode[]
  const args = children(ast.arguments)

  // `() => {...}` and `function () {...}` without a name behave the same here.
  if (
    (ast.type === 'ArrowFunctionExpression' && child(ast.body)?.type === 'BlockStatement') ||
    (ast.type === 'FunctionExpression' && !ast.id)
  ) {
    return {
      type: 'AnonymousFunction',
      params: canonicalizeNode(ast.params, closureNames),
      body: canonicalizeNode(ast.body, closureNames),
    }
  }

  // $localize`...`
  if (ast.type === 'TaggedTemplateExpression' && child(ast.tag)?.name === '$localize') {
    const quasi = child(ast.quasi)
    return {
      type: 'Localize',
      cooked: children(quasi?.quasis).map((part) => child(part.value)?.cooked),
      rawStrings: children(quasi?.quasis).map((part) => child(part.value)?.raw),
      substitutions: canonicalizeNode(quasi?.expressions, closureNames),
    }
  }

  // $localize(__makeTemplateObject([cooked], [raw]), ...substitutions)
  if (ast.type === 'CallExpression' && child(ast.callee)?.name === '$localize') {
    const [cookedArg, rawArg] = children(args[0]?.arguments)
    const cooked = stringElements(cookedArg)
    const raw = stringElements(rawArg)
    if (cooked && raw) {
      return {
        type: 'Localize',
        cooked,
        rawStrings: raw,
        substitutions: canonicalizeNode(args.slice(1), closureNames),
      }
    }
  }

  // goog.getMsg(message, params, { original_code: {...} }): the third argument is Closure-only.
  const callee = child(ast.callee)
  if (
    ast.type === 'CallExpression' &&
    callee?.type === 'MemberExpression' &&
    child(callee.object)?.name === 'goog' &&
    child(callee.property)?.name === 'getMsg' &&
    args.length === 3 &&
    args[2].type === 'ObjectExpression' &&
    children(args[2].properties).some((property) => child(property.key)?.name === 'original_code')
  ) {
    return canonicalizeNode({ ...ast, arguments: args.slice(0, 2) }, closureNames)
  }

  // Closure translation variable names: compare them by order of appearance.
  if (ast.type === 'Identifier' && typeof ast.name === 'string' && ast.name.startsWith('MSG_')) {
    if (!closureNames.has(ast.name)) {
      closureNames.set(ast.name, `MSG_${closureNames.size}`)
    }
    return { type: 'Identifier', name: closureNames.get(ast.name) }
  }

  // ɵɵdefinePipe({ pure: true }) is the default.
  if (ast.type === 'ObjectExpression') {
    const properties = children(ast.properties).filter(
      (property) =>
        !(child(property.key)?.name === 'pure' && child(property.value)?.value === true),
    )
    return { ...ast, properties: canonicalizeNode(properties, closureNames) }
  }

  const result: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(ast)) {
    result[key] = canonicalizeNode(value, closureNames)
  }
  return result
}

function compareStaticFields(
  oxcFields: StaticFieldAssignment[],
  tsFields: StaticFieldAssignment[],
): StaticFieldDiff[] {
  const diffs: StaticFieldDiff[] = []

  // Build maps by className.fieldName
  const oxcByKey = new Map<string, StaticFieldAssignment>()
  const tsByKey = new Map<string, StaticFieldAssignment>()

  for (const field of oxcFields) {
    const key = `${field.className}.${field.fieldName}`
    oxcByKey.set(key, field)
  }

  for (const field of tsFields) {
    const key = `${field.className}.${field.fieldName}`
    tsByKey.set(key, field)
  }

  // Find missing fields (in TS but not in Oxc)
  for (const [key, tsField] of tsByKey) {
    const oxcField = oxcByKey.get(key)
    if (!oxcField) {
      diffs.push({
        type: 'missing',
        className: tsField.className,
        fieldName: tsField.fieldName,
        expected: tsField.value,
      })
    } else if (!staticFieldValuesEquivalent(oxcField.node, tsField.node)) {
      diffs.push({
        type: 'different',
        className: tsField.className,
        fieldName: tsField.fieldName,
        expected: tsField.value,
        actual: oxcField.value,
      })
    }
  }

  // Find extra fields (in Oxc but not in TS)
  for (const [key, oxcField] of oxcByKey) {
    if (!tsByKey.has(key)) {
      diffs.push({
        type: 'extra',
        className: oxcField.className,
        fieldName: oxcField.fieldName,
        actual: oxcField.value,
      })
    }
  }

  return diffs
}

/**
 * Extract class declarations from the AST.
 */
interface ClassInfo {
  name: string
  normalized: string
}

/**
 * Extract class declarations from the program, exported or not. The `static ɵxxx` fields are
 * left out: they are compared on their own by `compareStaticFields`.
 */
function extractClasses(program: Program): Map<string, ClassInfo> {
  const classes = new Map<string, ClassInfo>()

  for (const topLevel of program.body as unknown as NormAstNode[]) {
    const statement =
      topLevel.type === 'ExportNamedDeclaration' || topLevel.type === 'ExportDefaultDeclaration'
        ? (topLevel.declaration as NormAstNode | null | undefined)
        : topLevel
    const name = (statement?.id as NormAstNode | null | undefined)?.name
    if (statement?.type !== 'ClassDeclaration' || typeof name !== 'string') continue
    const body = statement.body as NormAstNode
    const members = (body.body as NormAstNode[]).filter((member) => !isAngularStaticField(member))
    const normalized = normalizeAst({ ...statement, body: { ...body, body: members } })
    classes.set(name, { name, normalized })
  }

  return classes
}

/**
 * Compare class declarations between Oxc and TS outputs.
 */
function compareClasses(
  oxcClasses: Map<string, ClassInfo>,
  tsClasses: Map<string, ClassInfo>,
): ClassDiff[] {
  const diffs: ClassDiff[] = []

  // Find missing classes (in TS but not in Oxc)
  for (const [name, tsClass] of tsClasses) {
    const oxcClass = oxcClasses.get(name)
    if (!oxcClass) {
      diffs.push({
        type: 'missing',
        className: name,
        description: 'Class missing in Oxc output',
      })
    } else if (oxcClass.normalized !== tsClass.normalized) {
      diffs.push({
        type: 'different',
        className: name,
        description: 'Class definitions differ',
      })
    }
  }

  // Find extra classes (in Oxc but not in TS)
  for (const name of oxcClasses.keys()) {
    if (!tsClasses.has(name)) {
      diffs.push({
        type: 'extra',
        className: name,
        description: 'Extra class in Oxc output',
      })
    }
  }

  return diffs
}

/**
 * Compare two full-file JavaScript outputs semantically.
 *
 * This function performs a comprehensive comparison including:
 * - Import statements (using module.staticImports)
 * - Export statements (using module.staticExports)
 * - Class definitions
 * - Static field assignments (ClassName.ɵcmp, ClassName.ɵfac, etc.)
 * - Top-level functions and constants (template functions, hoisted `_cN` constants, etc.)
 *
 * Every part is compared on its AST, so formatting does not matter but any change inside a
 * string literal does; the source text is never rewritten to make the outputs line up.
 * Uses `showSemanticErrors: true` for additional validation.
 *
 * @param oxcCode - The Oxc-generated JavaScript code
 * @param tsCode - The TypeScript/Angular-generated JavaScript code
 * @returns A FullFileCompareResult with detailed comparison results
 */
export async function compareFullFileSemantically(
  oxcCode: string,
  tsCode: string,
): Promise<FullFileCompareResult> {
  // Fast path: if code strings are identical, they're semantically equivalent
  if (oxcCode === tsCode) {
    return { match: true }
  }

  try {
    // Format both with oxfmt. This does not change what is compared (the ASTs), but it lines
    // the reported source text up for readable diffs.
    const [formattedOxc, formattedTs] = await Promise.all([
      formatCodeForComparison(oxcCode),
      formatCodeForComparison(tsCode),
    ])

    // Fast path after formatting normalization
    if (formattedOxc === formattedTs) {
      return { match: true }
    }

    const parseOptions = { sourceType: 'module', showSemanticErrors: true } as const
    const oxcParsed = parseSync('oxc.js', formattedOxc, parseOptions)
    const tsParsed = parseSync('ts.js', formattedTs, parseOptions)

    // Check for parse errors
    const parseErrors: string[] = []
    if (oxcParsed.errors.length > 0) {
      parseErrors.push(...oxcParsed.errors.map((e) => `Oxc: ${e.message}`))
    }
    if (tsParsed.errors.length > 0) {
      parseErrors.push(...tsParsed.errors.map((e) => `TS: ${e.message}`))
    }

    if (parseErrors.length > 0) {
      return {
        match: false,
        parseErrors,
      }
    }

    // Rename Oxc's hoisted `_cN` constants to the Angular name of the constant with the same
    // value, so that both outputs use the same names when they hoist the same constants in a
    // different order. The constants themselves are still compared by name below.
    const constMapping = buildConstNameMapping(
      buildConstValueMap(oxcParsed.program as unknown as Program),
      buildConstValueMap(tsParsed.program as unknown as Program),
    )
    const normalizedOxcCode = replaceConstReferences(formattedOxc, constMapping)
    const normalizedTsCode = formattedTs

    // Fast path after const normalization
    if (normalizedOxcCode === normalizedTsCode) {
      return { match: true }
    }

    const oxcResult =
      normalizedOxcCode === formattedOxc
        ? oxcParsed
        : parseSync('oxc.js', normalizedOxcCode, parseOptions)
    const tsResult = tsParsed

    // Extract and compare imports using module.staticImports
    const oxcImports = extractNormalizedImports(oxcResult.module.staticImports)
    const tsImports = extractNormalizedImports(tsResult.module.staticImports)
    const importDiffs = compareImports(oxcImports, tsImports)

    // Extract and compare exports using module.staticExports
    const oxcExports = extractNormalizedExports(oxcResult.module.staticExports)
    const tsExports = extractNormalizedExports(tsResult.module.staticExports)
    const exportDiffs = compareExports(oxcExports, tsExports)

    // Extract and compare class definitions
    const oxcClasses = extractClasses(oxcResult.program as unknown as Program)
    const tsClasses = extractClasses(tsResult.program as unknown as Program)
    const classDiffs = compareClasses(oxcClasses, tsClasses)

    // Extract and compare the Angular static fields, both `ClassName.ɵcmp = ...` and the
    // `static ɵcmp = ...` class fields that current Angular and Oxc emit.
    const oxcFields = extractClassStaticFields(
      oxcResult.program as unknown as Program,
      normalizedOxcCode,
    )
    const tsFields = extractClassStaticFields(
      tsResult.program as unknown as Program,
      normalizedTsCode,
    )
    const staticFieldDiffs = compareStaticFields(oxcFields, tsFields)

    // Extract and compare class metadata (setClassMetadata calls)
    const oxcClassMetadata = extractClassMetadataCalls(oxcResult.program, normalizedOxcCode)
    const tsClassMetadata = extractClassMetadataCalls(tsResult.program, normalizedTsCode)
    const classMetadataDiffs = compareClassMetadata(oxcClassMetadata, tsClassMetadata)

    // Extract and compare top-level functions and constants (template functions, etc.)
    const oxcFunctions = extractFunctions(
      oxcResult.program as unknown as Program,
      normalizedOxcCode,
    )
    const tsFunctions = extractFunctions(tsResult.program as unknown as Program, normalizedTsCode)
    const functionComparison = compareFunctions(oxcFunctions, tsFunctions)

    // Determine if there are any differences
    const hasImportDiffs = importDiffs.length > 0
    const hasExportDiffs = exportDiffs.length > 0
    const hasClassDiffs = classDiffs.length > 0
    const hasStaticFieldDiffs = staticFieldDiffs.length > 0
    const hasClassMetadataDiffs = classMetadataDiffs.length > 0
    const hasFunctionDiffs =
      functionComparison.missingFunctions.length > 0 ||
      functionComparison.extraFunctions.length > 0 ||
      functionComparison.functionDiffs.length > 0

    const match =
      !hasImportDiffs &&
      !hasExportDiffs &&
      !hasClassDiffs &&
      !hasStaticFieldDiffs &&
      !hasClassMetadataDiffs &&
      !hasFunctionDiffs

    if (match) {
      return { match: true }
    }

    return {
      match: false,
      importDiffs: hasImportDiffs ? importDiffs : undefined,
      exportDiffs: hasExportDiffs ? exportDiffs : undefined,
      classDiffs: hasClassDiffs ? classDiffs : undefined,
      staticFieldDiffs: hasStaticFieldDiffs ? staticFieldDiffs : undefined,
      classMetadataDiffs: hasClassMetadataDiffs ? classMetadataDiffs : undefined,
      functionComparison: hasFunctionDiffs ? functionComparison : undefined,
    }
  } catch (e) {
    return {
      match: false,
      parseErrors: [`Error during comparison: ${e as Error}`],
    }
  }
}

/**
 * Format a full-file comparison result for display.
 */
export function formatFullFileComparison(result: FullFileCompareResult): string {
  if (result.match) {
    return 'Files match semantically.'
  }

  const lines: string[] = []

  if (result.parseErrors && result.parseErrors.length > 0) {
    lines.push('Parse errors:')
    for (const error of result.parseErrors) {
      lines.push(`  - ${error}`)
    }
    lines.push('')
  }

  if (result.importDiffs && result.importDiffs.length > 0) {
    lines.push('Import differences:')
    for (const diff of result.importDiffs) {
      switch (diff.type) {
        case 'missing':
          lines.push(
            `  - Missing: import { ${diff.expected?.join(', ')} } from "${diff.moduleSource}"`,
          )
          break
        case 'extra':
          lines.push(`  + Extra: import { ${diff.actual?.join(', ')} } from "${diff.moduleSource}"`)
          break
        case 'different':
          lines.push(`  ~ Different: "${diff.moduleSource}"`)
          lines.push(`      expected: { ${diff.expected?.join(', ')} }`)
          lines.push(`      actual:   { ${diff.actual?.join(', ')} }`)
          break
      }
    }
    lines.push('')
  }

  if (result.exportDiffs && result.exportDiffs.length > 0) {
    lines.push('Export differences:')
    for (const diff of result.exportDiffs) {
      switch (diff.type) {
        case 'missing':
          lines.push(`  - Missing export: ${diff.exportName}`)
          break
        case 'extra':
          lines.push(`  + Extra export: ${diff.exportName}`)
          break
        case 'different':
          lines.push(`  ~ Different export: ${diff.exportName}`)
          break
      }
    }
    lines.push('')
  }

  if (result.classDiffs && result.classDiffs.length > 0) {
    lines.push('Class differences:')
    for (const diff of result.classDiffs) {
      switch (diff.type) {
        case 'missing':
          lines.push(`  - Missing class: ${diff.className}`)
          break
        case 'extra':
          lines.push(`  + Extra class: ${diff.className}`)
          break
        case 'different':
          lines.push(`  ~ Different class: ${diff.className} - ${diff.description}`)
          break
      }
    }
    lines.push('')
  }

  if (result.staticFieldDiffs && result.staticFieldDiffs.length > 0) {
    lines.push('Static field differences:')
    for (const diff of result.staticFieldDiffs) {
      const fieldKey = `${diff.className}.${diff.fieldName}`
      switch (diff.type) {
        case 'missing':
          lines.push(`  - Missing: ${fieldKey}`)
          break
        case 'extra':
          lines.push(`  + Extra: ${fieldKey}`)
          break
        case 'different':
          lines.push(`  ~ Different: ${fieldKey}`)
          lines.push(`      expected: ${truncate(diff.expected || '', 80)}`)
          lines.push(`      actual:   ${truncate(diff.actual || '', 80)}`)
          break
      }
    }
    lines.push('')
  }

  if (result.classMetadataDiffs && result.classMetadataDiffs.length > 0) {
    lines.push('Class metadata (setClassMetadata) differences:')
    for (const diff of result.classMetadataDiffs) {
      const fieldKey = `${diff.className}.${diff.field}`
      switch (diff.type) {
        case 'missing':
          lines.push(`  - Missing: ${fieldKey}`)
          if (diff.expected) {
            lines.push(`      expected: ${truncate(diff.expected, 80)}`)
          }
          break
        case 'extra':
          lines.push(`  + Extra: ${fieldKey}`)
          if (diff.actual) {
            lines.push(`      actual: ${truncate(diff.actual, 80)}`)
          }
          break
        case 'different':
          lines.push(`  ~ Different: ${fieldKey}`)
          lines.push(`      expected: ${truncate(diff.expected || '', 80)}`)
          lines.push(`      actual:   ${truncate(diff.actual || '', 80)}`)
          break
      }
    }
    lines.push('')
  }

  if (result.functionComparison) {
    lines.push(formatFunctionComparison(result.functionComparison))
  }

  return lines.join('\n')
}

/**
 * Truncate a string to a maximum length.
 */
function truncate(str: string, maxLength: number): string {
  if (str.length <= maxLength) return str
  return str.slice(0, maxLength - 3) + '...'
}

/**
 * Common oxfmt options for normalizing comparison.
 * Both Oxc and Angular outputs should be formatted identically.
 */
const OXFMT_OPTIONS: FormatOptions = {
  semi: true,
  trailingComma: 'all',
  printWidth: 120,
  bracketSpacing: true,
  arrowParens: 'always',
  singleQuote: false,
  tabWidth: 2,
  useTabs: false,
  endOfLine: 'lf',
  insertFinalNewline: true,
  proseWrap: 'preserve',
  htmlWhitespaceSensitivity: 'css',
  vueIndentScriptAndStyle: false,
}

/**
 * Format code with oxfmt for comparison.
 * Returns the original code if formatting fails.
 */
async function formatCodeForComparison(code: string): Promise<string> {
  try {
    const result = await formatWithOxfmt('temp.js', code, OXFMT_OPTIONS)
    if (result.errors.length === 0) {
      return result.code
    }
    // Fall back to original code if formatting fails
    return code
  } catch {
    // Fall back to original code if formatting throws
    return code
  }
}

/**
 * AST node type used for traversal in normalization functions.
 */
interface NormAstNode {
  type?: string
  value?: unknown
  start?: number
  end?: number
  [key: string]: unknown
}

/**
 * Recursively walk an AST node, calling the visitor for each object node.
 */
function walkAst(node: unknown, visitor: (n: NormAstNode) => void): void {
  if (node === null || typeof node !== 'object') return
  if (Array.isArray(node)) {
    for (const item of node) walkAst(item, visitor)
    return
  }
  const obj = node as NormAstNode
  visitor(obj)
  for (const value of Object.values(obj)) {
    if (value !== null && typeof value === 'object') {
      walkAst(value, visitor)
    }
  }
}

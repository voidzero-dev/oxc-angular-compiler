import { diffLines } from 'diff'
import { parseSync, type StaticImport, type StaticExport } from 'oxc-parser'
import { format as formatWithOxfmt, type FormatOptions } from 'oxfmt'

import { normalizeTemplateLiterals } from './normalize-template-literals.js'
import type { FunctionLevelComparison, FunctionDiff, AstDiff, ClassMetadataDiff } from './types.js'

/**
 * Result of comparing two pieces of JavaScript code semantically.
 */
export interface CompareResult {
  /** Whether the two pieces of code are semantically equivalent */
  match: boolean
  /** Function-level comparison details */
  functionComparison?: FunctionLevelComparison
  /** Legacy differences found (deprecated) */
  diff?: AstDiff[]
}

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
 * Unwrap ParenthesizedExpression nodes since they don't affect semantics.
 * This normalizes `(expr)` to just `expr`.
 */
function unwrapParenthesized(node: unknown): unknown {
  if (node === null || typeof node !== 'object') {
    return node
  }

  const obj = node as Record<string, unknown>

  // Unwrap ParenthesizedExpression
  if (obj.type === 'ParenthesizedExpression' && 'expression' in obj) {
    return unwrapParenthesized(obj.expression)
  }

  // Recursively process arrays
  if (Array.isArray(node)) {
    return node.map(unwrapParenthesized)
  }

  // Recursively process object properties
  const result: Record<string, unknown> = {}
  for (const [key, value] of Object.entries(obj)) {
    result[key] = unwrapParenthesized(value)
  }
  return result
}

/**
 * Normalize AST to a canonical JSON string representation.
 * This excludes location and cosmetic properties, and unwraps parenthesized expressions.
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
 * Build a mapping from const names (_c0, _c1, etc.) to their normalized values.
 * This allows us to compare consts by value rather than by name.
 *
 * Handles both array constants and function constants:
 * - Array: const _c0 = ["value"];
 * - Function: const _c1 = (a0) => ({...});
 */
function buildConstValueMap(code: string): Map<string, string> {
  const constMap = new Map<string, string>()
  const constNamePattern = /^_c\d+$/

  // Parse the code using AST
  const ast = parseSync('file.js', code, { sourceType: 'module' })

  for (const stmt of ast.program.body) {
    // Only process const declarations
    if (
      stmt.type !== 'VariableDeclaration' ||
      stmt.kind !== 'const' ||
      stmt.declarations.length !== 1
    ) {
      continue
    }

    const decl = stmt.declarations[0]

    // Check if identifier matches _c\d+ pattern
    if (decl.id.type !== 'Identifier' || !constNamePattern.test(decl.id.name)) {
      continue
    }

    const constName = decl.id.name
    const init = decl.init

    if (!init) {
      continue
    }

    // Handle ArrayExpression: const _c0 = ["value"];
    if (init.type === 'ArrayExpression') {
      const constValue = code.slice(init.start, init.end)
      // Normalize the value by removing ALL whitespace for canonical comparison
      // This handles formatting differences (Oxc uses newlines/tabs, NG uses inline)
      const normalizedValue = constValue
        .replace(/\s+/g, '') // Remove ALL whitespace
        .trim()
      constMap.set(constName, normalizedValue)
    }
    // Handle ArrowFunctionExpression: const _c1 = (a0) => ({...});
    else if (init.type === 'ArrowFunctionExpression') {
      const constValue = code.slice(init.start, init.end)
      // Normalize the value for comparison:
      // 1. Remove all whitespace to get a canonical form
      // 2. Normalize arrow function syntax variations
      const normalizedValue = constValue
        .replace(/\s+/g, '') // Remove ALL whitespace
        // Normalize single param without parens: "a0=>" to "(a0)=>"
        .replace(/^([a-zA-Z_]\w*)=>/g, '($1)=>')
        .trim()
      constMap.set(constName, normalizedValue)
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

/**
 * Check if a const value is component metadata that Oxc's template compiler doesn't emit.
 *
 * These patterns are generated by Angular's full compiler for component metadata,
 * but Oxc's template-only compiler doesn't emit them:
 *
 * 1. Attrs arrays: ["selectorName", ""] - for component selectors/inputs
 *    Example: ["matButton", ""], ["bitPrefix", ""]
 *
 * 2. View query predicates: ["refName"] - single-element arrays for view/content queries
 *    Used by @ViewChild("refName"), @ViewChildren("refName"), @ContentChild, @ContentChildren
 *    Example: ["chipSelectButton"], ["prefixContainer"]
 *
 * Both are component metadata that only appears in TypeScript Angular's full-file output.
 */
function isAttrsConst(value: string): boolean {
  // Parse the value as an expression using oxc-parser
  const parseResult = parseSync('expr.js', value, { sourceType: 'script' })

  // Check if parsing succeeded and we have exactly one statement
  const program = parseResult.program
  if (program.body.length !== 1) {
    return false
  }

  const stmt = program.body[0]
  if (stmt.type !== 'ExpressionStatement') {
    return false
  }

  const expr = stmt.expression
  if (expr.type !== 'ArrayExpression') {
    return false
  }

  const elements = expr.elements

  // Helper to check if an element is a string literal
  const isStringLiteral = (el: unknown): boolean => {
    if (el && typeof el === 'object' && 'type' in el && 'value' in el) {
      return el.type === 'Literal' && typeof el.value === 'string'
    }
    return false
  }

  // Helper to get string value from a literal element
  const getStringValue = (el: unknown): string | undefined => {
    if (el && typeof el === 'object' && 'type' in el && 'value' in el) {
      if (el.type === 'Literal' && typeof el.value === 'string') {
        return el.value
      }
    }
    return undefined
  }

  // Attrs arrays: ["matButton", ""] or ["mat-fab", ""]
  // Pattern: 2 elements, both string Literals, second one is empty string
  if (elements.length === 2) {
    const first = elements[0]
    const second = elements[1]
    if (isStringLiteral(first) && isStringLiteral(second) && getStringValue(second) === '') {
      return true
    }
  }

  // View query predicates: ["refName"] - single-element string arrays
  // Used for @ViewChild("refName"), @ViewChildren("refName"), @ContentChild, @ContentChildren
  // These are NOT emitted by Oxc's template compiler because queries are class-level metadata
  // Pattern: 1 element which is a string Literal
  if (elements.length === 1) {
    const first = elements[0]
    if (isStringLiteral(first)) {
      return true
    }
  }

  return false
}

/**
 * Remove attrs/query consts from TS code that are NOT present in Oxc output.
 *
 * This function removes component metadata consts (like view query predicates) that only
 * appear in TypeScript Angular's full-file output because Oxc's template compiler doesn't
 * emit them.
 *
 * IMPORTANT: Only removes consts that are in TS but NOT in Oxc. If both outputs have the
 * same const value (e.g., selector arrays that Oxc also emits), it is NOT removed.
 *
 * After removal, remaining constants are renumbered to start from _c0 to align
 * with Oxc's numbering.
 *
 * Uses AST-based removal for correctness with nested structures.
 */
function removeAttrsConstsNotInOxc(
  tsCode: string,
  tsConstMap: Map<string, string>,
  oxcConstMap: Map<string, string>,
): string {
  // Build set of const VALUES present in Oxc output
  const oxcConstValues = new Set(oxcConstMap.values())

  // Find attrs consts in TS that are NOT in Oxc
  const attrsConstsToRemove = new Set<string>()
  for (const [name, value] of tsConstMap) {
    // Only remove if:
    // 1. The const looks like attrs/query metadata
    // 2. The same value does NOT exist in Oxc output
    if (isAttrsConst(value) && !oxcConstValues.has(value)) {
      attrsConstsToRemove.add(name)
    }
  }

  if (attrsConstsToRemove.size === 0) {
    return tsCode
  }

  // Remove the const declarations using AST
  let result = removeConstDeclarations(tsCode, attrsConstsToRemove)

  // Renumber remaining constants to start from _c0
  // This ensures Angular's output matches Oxc's constant numbering
  const remainingConsts: string[] = []
  for (const name of tsConstMap.keys()) {
    if (!attrsConstsToRemove.has(name)) {
      remainingConsts.push(name)
    }
  }

  // Sort remaining constants by their numeric suffix
  remainingConsts.sort((a, b) => {
    const numA = parseInt(a.replace('_c', ''), 10)
    const numB = parseInt(b.replace('_c', ''), 10)
    return numA - numB
  })

  // Build rename map: old name -> new name (starting from _c0)
  const renameMap = new Map<string, string>()
  for (let i = 0; i < remainingConsts.length; i++) {
    const oldName = remainingConsts[i]
    const newName = `_c${i}`
    if (oldName !== newName) {
      renameMap.set(oldName, newName)
    }
  }

  // Replace const references using the rename map
  if (renameMap.size > 0) {
    result = replaceConstReferences(result, renameMap)
  }

  return result
}

/**
 * Remove const declarations by name using AST-based approach.
 * This correctly handles any const value structure including nested arrays/objects.
 */
function removeConstDeclarations(code: string, constNamesToRemove: Set<string>): string {
  const ast = parseSync('file.js', code, { sourceType: 'module' })

  // Collect spans to remove (sorted by start position descending for safe removal)
  const spansToRemove: Array<{ start: number; end: number }> = []

  for (const stmt of ast.program.body) {
    if (
      stmt.type === 'VariableDeclaration' &&
      stmt.kind === 'const' &&
      stmt.declarations.length === 1
    ) {
      const decl = stmt.declarations[0]
      if (decl.id.type === 'Identifier' && constNamesToRemove.has(decl.id.name)) {
        spansToRemove.push({ start: stmt.start, end: stmt.end })
      }
    }
  }

  if (spansToRemove.length === 0) {
    return code
  }

  // Sort by start position descending so we can remove from end to start
  // without affecting earlier positions
  spansToRemove.sort((a, b) => b.start - a.start)

  let result = code
  for (const span of spansToRemove) {
    // Also remove trailing whitespace/newline after the statement
    let end = span.end
    while (end < result.length && (result[end] === ' ' || result[end] === '\n')) {
      end++
    }
    result = result.slice(0, span.start) + result.slice(end)
  }

  return result
}

/**
 * Remove provider const declarations from code.
 * Providers are component metadata, not template output, and may contain
 * expressions the Angular TS compiler can't handle (showing as "/* unknown * /").
 *
 * Uses AST-based removal for correctness.
 */
function removeProvidersConst(code: string): string {
  return removeConstDeclarations(code, new Set(['_providers']))
}

/**
 * Create a byte-offset to character-index mapping for a string.
 * This pre-computes the mapping once for efficient lookups.
 */
function createByteToCharMap(str: string): number[] {
  // For ASCII-only strings, byte offset === char index
  // Quick check: if all chars are < 128, return empty map as sentinel
  let hasNonAscii = false
  for (let i = 0; i < str.length; i++) {
    if (str.charCodeAt(i) >= 128) {
      hasNonAscii = true
      break
    }
  }
  if (!hasNonAscii) {
    return [] // Empty sentinel means use identity mapping
  }

  const encoder = new TextEncoder()
  const map: number[] = [0] // byte 0 -> char 0
  let byteCount = 0

  for (let i = 0; i < str.length; i++) {
    const code = str.charCodeAt(i)
    // Handle surrogate pairs (characters outside BMP)
    // High surrogate: 0xD800-0xDBFF, Low surrogate: 0xDC00-0xDFFF
    // Only treat as a pair if followed by a valid low surrogate
    const isHighSurrogate = code >= 0xd800 && code <= 0xdbff
    const nextCode = i + 1 < str.length ? str.charCodeAt(i + 1) : 0
    const isValidPair = isHighSurrogate && nextCode >= 0xdc00 && nextCode <= 0xdfff

    if (isValidPair) {
      const pair = str.slice(i, i + 2)
      const bytes = encoder.encode(pair).length
      for (let b = 0; b < bytes; b++) {
        byteCount++
        map[byteCount] = i + 2 // Next char after the pair
      }
      i++ // Skip the low surrogate
    } else {
      // Single character (including unpaired surrogates)
      const bytes = encoder.encode(str[i]).length
      for (let b = 0; b < bytes; b++) {
        byteCount++
        map[byteCount] = i + 1 // Next char
      }
    }
  }

  return map
}

/**
 * Convert a byte offset to a string character index using a pre-computed map.
 */
function byteOffsetToCharIndex(
  byteToCharMap: number[],
  byteOffset: number,
  strLength: number,
): number {
  // Empty map means ASCII-only, use identity mapping
  if (byteToCharMap.length === 0) {
    return Math.min(byteOffset, strLength)
  }
  if (byteOffset >= byteToCharMap.length) {
    return strLength
  }
  return byteToCharMap[byteOffset] ?? strLength
}

/**
 * Extract a single function's code from the source by its AST bounds.
 * Handles byte offset to character index conversion for non-ASCII support.
 */
function extractFunctionCode(code: string, funcAst: FunctionAst, byteToCharMap: number[]): string {
  // oxc-parser uses 'start' and 'end' directly on nodes (not nested in 'span')
  // Try both patterns for compatibility
  let start: number | undefined
  let end: number | undefined

  if (
    funcAst.span &&
    typeof funcAst.span.start === 'number' &&
    typeof funcAst.span.end === 'number'
  ) {
    start = funcAst.span.start
    end = funcAst.span.end
  } else if (typeof funcAst.start === 'number' && typeof funcAst.end === 'number') {
    start = funcAst.start
    end = funcAst.end
  }

  if (start !== undefined && end !== undefined) {
    const startIdx = byteOffsetToCharIndex(byteToCharMap, start, code.length)
    const endIdx = byteOffsetToCharIndex(byteToCharMap, end, code.length)
    return code.slice(startIdx, endIdx)
  }

  // Fallback: re-stringify the AST (less accurate but works)
  return JSON.stringify(funcAst)
}

interface FunctionAst {
  type: 'FunctionDeclaration' | 'FunctionExpression' | 'ArrowFunctionExpression'
  id?: { name: string } | null
  span?: { start: number; end: number }
  start?: number
  end?: number
  [key: string]: unknown
}

interface VariableDeclaration {
  type: 'VariableDeclaration'
  declarations: Array<{
    type: 'VariableDeclarator'
    id: { type: string; name?: string } | null
    init: { type: string; [key: string]: unknown } | null
    span?: { start: number; end: number }
    start?: number
    end?: number
  }>
  span?: { start: number; end: number }
  start?: number
  end?: number
  [key: string]: unknown
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

/**
 * Normalize constructor parameter types for @Inject decorated parameters.
 *
 * When a constructor parameter has an @Inject decorator with an injection token
 * (like DOCUMENT), the Angular compiler outputs `{type:undefined}` because the
 * token is not a type. Oxc may preserve the actual type annotation (e.g., Document).
 *
 * This normalizes both outputs to use `type:undefined` for @Inject parameters,
 * which is the canonical form used by Angular's setClassMetadata.
 *
 * Example:
 *   Input:  {type:Document,decorators:[{type:Inject,args:[DOCUMENT]}]}
 *   Output: {type:undefined,decorators:[{type:Inject,args:[DOCUMENT]}]}
 */
function normalizeInjectParameterTypes(code: string): string {
  // Match patterns like: {type:SomeType,decorators:[{type:Inject, or {type:Inject,args:
  // and replace the type with undefined if it has an @Inject decorator
  //
  // Pattern matches: {type:IDENTIFIER,decorators:[{type:Inject
  // Captures the identifier and replaces with undefined
  return code.replace(
    /\{type:([A-Z][A-Za-z0-9]*),decorators:\[\{type:Inject/g,
    '{type:undefined,decorators:[{type:Inject',
  )
}

// AST node types for nullish coalescing pattern matching
interface NullishAstNode {
  type: string
  start: number
  end: number
  [key: string]: unknown
}

interface NullishConditionalExpressionNode extends NullishAstNode {
  type: 'ConditionalExpression'
  test: NullishAstNode
  consequent: NullishAstNode
  alternate: NullishAstNode
}

interface NullishLogicalExpressionNode extends NullishAstNode {
  type: 'LogicalExpression'
  operator: string
  left: NullishAstNode
  right: NullishAstNode
}

interface NullishBinaryExpressionNode extends NullishAstNode {
  type: 'BinaryExpression'
  operator: string
  left: NullishAstNode
  right: NullishAstNode
}

interface NullishAssignmentExpressionNode extends NullishAstNode {
  type: 'AssignmentExpression'
  operator: string
  left: NullishAstNode
  right: NullishAstNode
}

interface NullishIdentifierNode extends NullishAstNode {
  type: 'Identifier'
  name: string
}

interface NullishUnaryExpressionNode extends NullishAstNode {
  type: 'UnaryExpression'
  operator: string
  argument: NullishAstNode
}

interface NullishLiteralNode extends NullishAstNode {
  type: 'Literal'
  value: unknown
}

interface NullishParenthesizedExpressionNode extends NullishAstNode {
  type: 'ParenthesizedExpression'
  expression: NullishAstNode
}

interface NullishVariableDeclarationNode extends NullishAstNode {
  type: 'VariableDeclaration'
  kind: string
  declarations: NullishVariableDeclaratorNode[]
}

interface NullishVariableDeclaratorNode extends NullishAstNode {
  type: 'VariableDeclarator'
  id: NullishAstNode
}

/**
 * Check if an AST node is a nullish coalescing pattern:
 *   Pattern 1: (_a = expr) !== null && _a !== void 0 ? _a : fallback
 *   Pattern 2: expr !== null && expr !== void 0 ? expr : fallback
 *
 * Returns the extracted parts if it matches, null otherwise.
 */
function matchNullishCoalescingPattern(
  node: NullishAstNode,
  code: string,
): { leftExpr: string; rightExpr: string; tempVarName: string } | null {
  // Must be a ConditionalExpression
  if (node.type !== 'ConditionalExpression') return null
  const conditional = node as NullishConditionalExpressionNode

  // test must be a LogicalExpression with && operator
  if (conditional.test.type !== 'LogicalExpression') return null
  const testLogical = conditional.test as NullishLogicalExpressionNode
  if (testLogical.operator !== '&&') return null

  // Left side of &&: (_a = expr) !== null  OR  expr !== null
  // This should be a BinaryExpression with !== operator
  if (testLogical.left.type !== 'BinaryExpression') return null
  const leftBinary = testLogical.left as NullishBinaryExpressionNode
  if (leftBinary.operator !== '!==' && leftBinary.operator !== '!=') return null

  // Right side of !== should be null (Literal with value: null in oxc-parser)
  if (leftBinary.right.type !== 'Literal') return null
  const nullLiteral = leftBinary.right as NullishLiteralNode
  if (nullLiteral.value !== null) return null

  // Left side of !== could be:
  // 1. ParenthesizedExpression containing AssignmentExpression (temp var pattern)
  // 2. Just an Identifier or other expression (simple pattern)
  let leftOfBinary = leftBinary.left

  // Unwrap parentheses if needed
  if (leftOfBinary.type === 'ParenthesizedExpression') {
    leftOfBinary = (leftOfBinary as NullishParenthesizedExpressionNode).expression
  }

  // Right side of &&: _a !== void 0  OR  expr !== void 0
  if (testLogical.right.type !== 'BinaryExpression') return null
  const rightBinary = testLogical.right as NullishBinaryExpressionNode
  if (rightBinary.operator !== '!==' && rightBinary.operator !== '!=') return null

  // Right side should be "void 0"
  if (rightBinary.right.type !== 'UnaryExpression') return null
  const voidExpr = rightBinary.right as NullishUnaryExpressionNode
  if (voidExpr.operator !== 'void') return null
  // In oxc-parser, numeric literals are "Literal" with numeric value
  if (voidExpr.argument.type !== 'Literal') return null
  const voidArg = voidExpr.argument as NullishLiteralNode
  if (voidArg.value !== 0) return null

  // Try to match Pattern 1: (_a = expr) !== null && _a !== void 0 ? _a : fallback
  if (leftOfBinary.type === 'AssignmentExpression') {
    const assignmentExpr = leftOfBinary as NullishAssignmentExpressionNode

    // Assignment should be to a temp variable like _a, _b, etc.
    if (assignmentExpr.left.type !== 'Identifier') return null
    const tempVar = assignmentExpr.left as NullishIdentifierNode
    const tempVarName = tempVar.name

    // Check that it's a temp variable pattern (starts with _)
    if (!tempVarName.startsWith('_')) return null

    // Left side of !== void 0 should be the same temp variable
    if (rightBinary.left.type !== 'Identifier') return null
    const rightVarRef = rightBinary.left as NullishIdentifierNode
    if (rightVarRef.name !== tempVarName) return null

    // consequent should be the same temp variable
    let consequent = conditional.consequent
    if (consequent.type === 'ParenthesizedExpression') {
      consequent = (consequent as NullishParenthesizedExpressionNode).expression
    }
    if (consequent.type !== 'Identifier') return null
    const consequentVar = consequent as NullishIdentifierNode
    if (consequentVar.name !== tempVarName) return null

    // Extract the left expression (the actual value being null-checked)
    const leftExpr = code.slice(assignmentExpr.right.start, assignmentExpr.right.end)

    // Extract the right expression (the fallback)
    const rightExpr = code.slice(conditional.alternate.start, conditional.alternate.end)

    return { leftExpr, rightExpr, tempVarName }
  }

  // Try to match Pattern 2: expr !== null && expr !== void 0 ? expr : fallback
  // Both sides of && compare the same expression
  const leftExprStr = code.slice(leftBinary.left.start, leftBinary.left.end)
  const rightExprStr = code.slice(rightBinary.left.start, rightBinary.left.end)

  // The expressions must be the same
  if (leftExprStr !== rightExprStr) return null

  // The consequent must also be the same expression
  let consequent = conditional.consequent
  if (consequent.type === 'ParenthesizedExpression') {
    consequent = (consequent as NullishParenthesizedExpressionNode).expression
  }
  const consequentStr = code.slice(consequent.start, consequent.end)
  if (consequentStr !== leftExprStr) return null

  // Extract the right expression (the fallback)
  const rightExpr = code.slice(conditional.alternate.start, conditional.alternate.end)

  return { leftExpr: leftExprStr, rightExpr, tempVarName: '' }
}

/**
 * Recursively walk an AST and collect all nullish coalescing patterns.
 * Returns them sorted by start position descending for safe replacement.
 */
function collectNullishPatterns(
  node: NullishAstNode,
  code: string,
  patterns: Array<{ start: number; end: number; leftExpr: string; rightExpr: string }>,
): void {
  // Try to match this node as a nullish coalescing pattern
  const match = matchNullishCoalescingPattern(node, code)
  if (match) {
    patterns.push({
      start: node.start,
      end: node.end,
      leftExpr: match.leftExpr,
      rightExpr: match.rightExpr,
    })
    // Don't recurse into children of matched patterns - nested patterns
    // within leftExpr/rightExpr are handled by recursive normalization
    // in the replacement loop. Collecting them here would cause position
    // conflicts since the outer pattern's span contains the inner pattern.
    return
  }

  // Recursively walk all child nodes (only for non-matched nodes)
  for (const key of Object.keys(node)) {
    const value = node[key]
    if (value && typeof value === 'object') {
      if (Array.isArray(value)) {
        for (const item of value) {
          if (item && typeof item === 'object' && 'type' in item) {
            collectNullishPatterns(item as NullishAstNode, code, patterns)
          }
        }
      } else if ('type' in value) {
        collectNullishPatterns(value as NullishAstNode, code, patterns)
      }
    }
  }
}

/**
 * Collect all "var _a, _b, ...;" declarations (temp variables for nullish coalescing).
 * Returns spans sorted by start position descending for safe removal.
 */
function collectTempVarDeclarations(
  ast: { program: { body: unknown[] } },
  code: string,
): Array<{ start: number; end: number }> {
  const spans: Array<{ start: number; end: number }> = []

  for (const stmt of ast.program.body) {
    const stmtNode = stmt as NullishAstNode
    if (stmtNode.type === 'VariableDeclaration') {
      const varDecl = stmtNode as NullishVariableDeclarationNode
      if (varDecl.kind === 'var') {
        // Check if ALL declarators are temp variables for nullish coalescing.
        // TypeScript generates temp vars like _a, _b, _c (single underscore + letter/number).
        // We must NOT remove helpers like __awaiter, __generator (double underscore).
        const isTempVar = (name: string): boolean => {
          // Match _a, _b, ..., _z, _1, _2, etc. but NOT __awaiter, __generator, etc.
          return /^_[a-z0-9]$/i.test(name)
        }
        const allTempVars = varDecl.declarations.every((decl) => {
          if (decl.id.type === 'Identifier') {
            const id = decl.id as NullishIdentifierNode
            return isTempVar(id.name)
          }
          return false
        })

        if (allTempVars && varDecl.declarations.length > 0) {
          // Include trailing whitespace/newline in removal
          let end = stmtNode.end
          while (end < code.length && (code[end] === ' ' || code[end] === '\n')) {
            end++
          }
          spans.push({ start: stmtNode.start, end })
        }
      }
    }
  }

  return spans
}

/**
 * Check if an expression contains a logical operator (|| or &&) at ANY level.
 * This helps determine if the expression needs parentheses when used with ??.
 *
 * For example, `x === null || x === void 0 ? void 0 : x.y` has a ConditionalExpression
 * at the top level, but contains a LogicalExpression inside. Mixing this with ??
 * would produce invalid code, so we need to detect logical operators at any depth.
 */
function containsLogicalOperator(expr: string): boolean {
  try {
    const ast = parseSync('expr.js', expr, { sourceType: 'module' })
    const program = ast.program
    if (program.body.length !== 1) return false
    const stmt = program.body[0]
    if (stmt.type !== 'ExpressionStatement') return false
    const stmtExpr = (stmt as unknown as { expression: NullishAstNode }).expression
    return hasLogicalOperatorRecursive(stmtExpr)
  } catch {
    // Fallback to string check if parsing fails
    return expr.includes('||') || expr.includes('&&')
  }
}

/**
 * Recursively traverse an AST node to find any LogicalExpression with || or &&.
 */
function hasLogicalOperatorRecursive(node: NullishAstNode): boolean {
  if (!node || typeof node !== 'object') return false

  // Check if this node is a LogicalExpression with || or &&
  if (node.type === 'LogicalExpression') {
    const op = (node as unknown as { operator: string }).operator
    if (op === '||' || op === '&&') {
      return true
    }
  }

  // Recursively check all child nodes
  for (const key of Object.keys(node)) {
    if (key === 'type' || key === 'start' || key === 'end' || key === 'span') continue
    const value = (node as Record<string, unknown>)[key]
    if (Array.isArray(value)) {
      for (const item of value) {
        if (
          item &&
          typeof item === 'object' &&
          hasLogicalOperatorRecursive(item as NullishAstNode)
        ) {
          return true
        }
      }
    } else if (value && typeof value === 'object') {
      if (hasLogicalOperatorRecursive(value as NullishAstNode)) {
        return true
      }
    }
  }

  return false
}

/**
 * Normalize nullish coalescing operator differences using AST-based transformation.
 *
 * Oxc preserves the ?? operator while TypeScript transpiles it to:
 *   (_a = expr) !== null && _a !== void 0 ? _a : fallback
 *
 * This function converts the TypeScript transpiled pattern back to ?? syntax
 * so both can be compared equivalently.
 *
 * Example:
 *   Input:  (_a = expr) !== null && _a !== void 0 ? _a : fallback
 *   Output: expr ?? fallback
 *
 * The pattern handles:
 * - Variable names like _a, _b, _c, etc.
 * - Nested expressions as the left operand (including property access and function calls)
 * - Various fallback expressions (including function calls)
 * - Nested nullish coalescing patterns
 */
function normalizeNullishCoalescing(code: string): string {
  // Parse the code to get AST
  let ast
  try {
    ast = parseSync('nullish.js', code, { sourceType: 'module' })
  } catch {
    // If parsing fails, return code unchanged
    return code
  }

  // Collect all temp var declarations to remove
  const tempVarSpans = collectTempVarDeclarations(ast, code)

  // Collect all nullish coalescing patterns
  const patterns: Array<{ start: number; end: number; leftExpr: string; rightExpr: string }> = []
  collectNullishPatterns(ast.program as unknown as NullishAstNode, code, patterns)

  // If nothing to transform, return as-is
  if (patterns.length === 0 && tempVarSpans.length === 0) {
    return code
  }

  // Combine all spans and sort by start position descending for safe replacement
  const allReplacements: Array<{
    start: number
    end: number
    replacement: string
  }> = []

  // Add pattern replacements
  for (const pattern of patterns) {
    // Handle nested patterns by recursively normalizing leftExpr
    let normalizedLeft = pattern.leftExpr
    if (normalizedLeft.includes('!== null') && normalizedLeft.includes('!== void 0')) {
      normalizedLeft = normalizeNullishCoalescing(normalizedLeft)
    }

    // Also normalize the rightExpr in case it has nested patterns
    let normalizedRight = pattern.rightExpr
    if (normalizedRight.includes('!== null') && normalizedRight.includes('!== void 0')) {
      normalizedRight = normalizeNullishCoalescing(normalizedRight)
    }

    // Determine if we need parentheses around the left expression
    // Parse leftExpr to check if it contains || or &&
    const leftNeedsParens = containsLogicalOperator(normalizedLeft)
    const formattedLeft = leftNeedsParens ? `(${normalizedLeft})` : normalizedLeft

    // Also check if right expression needs parentheses
    // This handles cases like: x ?? y?.prop where y?.prop is transpiled as (y === null || ...)
    // Also check for yield/await which have low precedence and need parens when on RHS of ??
    const rightNeedsParens =
      containsLogicalOperator(normalizedRight) || /^(yield|await)\b/.test(normalizedRight.trim())
    const formattedRight = rightNeedsParens ? `(${normalizedRight})` : normalizedRight

    // Check if the surrounding context has && or || operators
    // If so, we need to wrap the entire ?? expression in parentheses
    const contextNeedsParens = hasMixedLogicalInContext(code, pattern.start, pattern.end)
    const replacement = `${formattedLeft} ?? ${formattedRight}`
    const finalReplacement = contextNeedsParens ? `(${replacement})` : replacement

    allReplacements.push({
      start: pattern.start,
      end: pattern.end,
      replacement: finalReplacement,
    })
  }

  // Add temp var removal (empty replacement)
  for (const span of tempVarSpans) {
    allReplacements.push({
      start: span.start,
      end: span.end,
      replacement: '',
    })
  }

  // Sort by start position descending
  allReplacements.sort((a, b) => b.start - a.start)

  // Apply replacements from end to start
  let result = code
  for (const rep of allReplacements) {
    result = result.slice(0, rep.start) + rep.replacement + result.slice(rep.end)
  }

  return result
}

/**
 * Remove parentheses around nullish coalescing expressions for normalization.
 *
 * Oxc wraps nullish coalescing in parens: (expr ?? fallback)
 * After TS normalization, it becomes: expr ?? fallback
 *
 * This function removes the outer parens from Oxc's output to match.
 * It is CONSERVATIVE - only removes parens when it's provably safe.
 */
function removeNullishCoalescingParens(code: string): string {
  // Use a manual approach to handle balanced parentheses
  // Look for patterns like (expr ?? expr) and remove the outer parens
  let output = ''
  let i = 0

  while (i < code.length) {
    if (code[i] === '(') {
      // Try to match a nullish coalescing expression wrapped in parens
      const matchResult = tryRemoveNullishParens(code, i)
      if (matchResult) {
        output += matchResult.replacement
        i = matchResult.endIndex
        continue
      }
    }
    output += code[i]
    i++
  }

  return output
}

/**
 * Try to remove outer parens from a nullish coalescing expression starting at position `start`.
 * Returns { replacement, endIndex } if matched, null otherwise.
 *
 * This function is CONSERVATIVE - it only removes parentheses when:
 * 1. The inner content parses as a single BinaryExpression with `??` operator
 * 2. There are no commas at the top level (would indicate function args)
 * 3. There are no `&&` or `||` operators ANYWHERE in the surrounding statement context
 * 4. The parentheses are truly just wrapping a standalone `??` expression
 */
function tryRemoveNullishParens(
  code: string,
  start: number,
): { replacement: string; endIndex: number } | null {
  // We're at a '(' - find the matching ')' and check if it contains ' ?? '
  let depth = 0
  let nullishCount = 0
  let hasTopLevelComma = false
  let end = -1

  for (let j = start; j < code.length; j++) {
    const ch = code[j]
    if (ch === '(') depth++
    else if (ch === ')') {
      depth--
      if (depth === 0) {
        end = j
        break
      }
    }
    // Check for ' ?? ' at depth 1 (inside our parens but not nested deeper)
    if (depth === 1 && code.slice(j, j + 4) === ' ?? ') {
      nullishCount++
    }
    // Check for comma at depth 1 - indicates function arguments, not wrapping parens
    if (depth === 1 && ch === ',') {
      hasTopLevelComma = true
    }
  }

  // Must have found closing paren, exactly one ?? at top level, and no commas
  if (end === -1 || nullishCount !== 1 || hasTopLevelComma) return null

  // Extract the inner content (between the outer parens)
  const inner = code.slice(start + 1, end)

  // Use AST parsing to verify the inner content is a single BinaryExpression with ??
  if (!isValidNullishExpressionForParenRemoval(inner)) {
    return null
  }

  // Check surrounding context for && or || operators
  // We need to look at the broader statement context, not just immediately adjacent
  if (hasMixedLogicalInContext(code, start, end)) {
    return null
  }

  return {
    replacement: inner,
    endIndex: end + 1,
  }
}

/**
 * Use AST parsing to verify the expression is a single BinaryExpression with ?? operator.
 * This ensures we're not accidentally mangling complex expressions.
 */
function isValidNullishExpressionForParenRemoval(expr: string): boolean {
  try {
    // Wrap in a minimal expression context for parsing
    const wrappedCode = `(${expr})`
    const result = parseSync('temp.js', wrappedCode, { sourceType: 'module' })

    if (result.errors.length > 0) {
      return false
    }

    const program = result.program
    if (program.body.length !== 1) return false

    const stmt = program.body[0]
    if (stmt.type !== 'ExpressionStatement') return false

    const expression = (stmt as unknown as { expression: NullishAstNode }).expression

    // Must be a BinaryExpression with ?? operator
    if (expression.type !== 'BinaryExpression') return false
    const binExpr = expression as unknown as {
      operator: string
      left: NullishAstNode
      right: NullishAstNode
    }
    if (binExpr.operator !== '??') return false

    // Check that neither side contains && or || at any level
    // This prevents issues like (a && b ?? c) which would be invalid without parens
    if (hasLogicalOperatorRecursive(binExpr.left) || hasLogicalOperatorRecursive(binExpr.right)) {
      return false
    }

    return true
  } catch {
    // If parsing fails, be conservative and don't remove parens
    return false
  }
}

/**
 * Check if the surrounding context contains && or || that would mix with ??.
 *
 * JavaScript doesn't allow mixing ?? with && or || without explicit parentheses.
 * So if there's any && or || in the same statement/expression context,
 * we must keep the parentheses.
 *
 * This function looks at the broader context by finding the statement boundaries.
 */
function hasMixedLogicalInContext(code: string, parenStart: number, parenEnd: number): boolean {
  // Find the start of the current statement/expression context
  // Look backwards for statement terminators or expression boundaries
  let contextStart = parenStart
  let depth = 0

  for (let i = parenStart - 1; i >= 0; i--) {
    const ch = code[i]
    if (ch === ')') depth++
    else if (ch === '(') {
      if (depth > 0) depth--
      else {
        // We're inside another paren group - this defines our context
        contextStart = i + 1
        break
      }
    } else if (ch === ';' || ch === '{' || ch === '}' || ch === '?' || ch === ':') {
      contextStart = i + 1
      break
    }
    if (i === 0) contextStart = 0
  }

  // Find the end of the current statement/expression context
  let contextEnd = parenEnd
  depth = 0

  for (let i = parenEnd + 1; i < code.length; i++) {
    const ch = code[i]
    if (ch === '(') depth++
    else if (ch === ')') {
      if (depth > 0) depth--
      else {
        // We're closing a paren that was opened before us
        contextEnd = i
        break
      }
    } else if (ch === ';' || ch === '{' || ch === '}' || ch === '?' || ch === ':') {
      contextEnd = i
      break
    }
    if (i === code.length - 1) contextEnd = code.length
  }

  // Now check for && or || in this context (excluding the content inside our parens)
  const beforeParen = code.slice(contextStart, parenStart)
  const afterParen = code.slice(parenEnd + 1, contextEnd)

  // Check for && or || in the surrounding context (not inside our parens)
  // We need to be careful about && and || that might be inside nested parens
  if (containsLogicalOperatorInString(beforeParen) || containsLogicalOperatorInString(afterParen)) {
    return true
  }

  return false
}

/**
 * Check if a code string contains && or || at its top level (not inside nested parens).
 */
function containsLogicalOperatorInString(code: string): boolean {
  let depth = 0

  for (let i = 0; i < code.length; i++) {
    const ch = code[i]
    if (ch === '(' || ch === '[' || ch === '{') depth++
    else if (ch === ')' || ch === ']' || ch === '}') depth--
    else if (depth === 0) {
      // Check for && or ||
      if (i + 1 < code.length) {
        const twoChar = code.slice(i, i + 2)
        if (twoChar === '&&' || twoChar === '||') {
          return true
        }
      }
    }
  }

  return false
}

/**
 * Compare two JavaScript code strings semantically using normalized string comparison.
 *
 * This parses both strings with oxc-parser, normalizes the ASTs (removing location info),
 * and compares the resulting canonical strings.
 *
 * It also normalizes const references (_c0, _c1, etc.) by value, so that const name
 * differences due to different ordering/numbering don't cause false mismatches.
 */
export async function compareJsSemantically(
  oxcCode: string,
  tsCode: string,
): Promise<CompareResult> {
  // Fast path: if code strings are identical, they're semantically equivalent
  if (oxcCode === tsCode) {
    return { match: true }
  }

  try {
    // Format both with oxfmt to normalize formatting
    const [formattedOxc, formattedTs] = await Promise.all([
      formatCodeForComparison(oxcCode),
      formatCodeForComparison(tsCode),
    ])

    // Fast path after formatting normalization
    if (formattedOxc === formattedTs) {
      return { match: true }
    }

    // Normalize template literals to regular strings (semantically equivalent)
    let workingOxcCode = normalizeTemplateLiterals(formattedOxc)
    let workingTsCode = normalizeTemplateLiterals(formattedTs)

    // Normalize PURE comments (oxfmt adds spaces: /* @__PURE__ */ -> /*@__PURE__*/)
    workingOxcCode = workingOxcCode.replaceAll('/* @__PURE__ */', '/*@__PURE__*/')
    workingTsCode = workingTsCode.replaceAll('/* @__PURE__ */', '/*@__PURE__*/')

    // Remove trailing commas in objects: ",}" -> "}"
    workingOxcCode = workingOxcCode.replaceAll(',}', '}')
    workingTsCode = workingTsCode.replaceAll(',}', '}')

    // Remove trailing commas in arrays: ",]" -> "]"
    workingOxcCode = workingOxcCode.replaceAll(',]', ']')
    workingTsCode = workingTsCode.replaceAll(',]', ']')

    // Normalize space before closing brace: " }" -> "}"
    workingOxcCode = workingOxcCode.replaceAll(' }', '}')
    workingTsCode = workingTsCode.replaceAll(' }', '}')

    // Normalize @Inject parameter types (Oxc preserves type, Angular uses undefined)
    workingOxcCode = normalizeInjectParameterTypes(workingOxcCode)
    workingTsCode = normalizeInjectParameterTypes(workingTsCode)

    // Normalize nullish coalescing (Oxc preserves ??, TS transpiles to ternary)
    // Apply to TS: convert ternary check to ??
    // Apply to Oxc: remove outer parens around ?? expression
    workingTsCode = normalizeNullishCoalescing(workingTsCode)
    workingOxcCode = removeNullishCoalescingParens(workingOxcCode)

    // Normalize numeric literals: 1e3 → 1000 (OXC emits scientific notation)
    workingOxcCode = normalizeNumericLiterals(workingOxcCode)
    workingTsCode = normalizeNumericLiterals(workingTsCode)

    // Normalize inline object literal formatting: collapse multi-line simple objects
    workingOxcCode = collapseInlineObjectLiterals(workingOxcCode)
    workingTsCode = collapseInlineObjectLiterals(workingTsCode)

    // Fast path after template/inject/nullish/format normalization
    if (workingOxcCode === workingTsCode) {
      return { match: true }
    }

    // Build const value maps for both to identify metadata consts
    const tsConstMap = buildConstValueMap(workingTsCode)
    const oxcConstMapInitial = buildConstValueMap(workingOxcCode)

    // Remove attrs/query consts from TS code that are NOT present in Oxc output.
    // These are component metadata consts (like view query predicates) that only appear
    // in TypeScript Angular's full-file output because Oxc's template compiler doesn't emit them.
    // If both outputs have the same const (e.g., selector arrays), don't remove it.
    let normalizedTsCode = removeAttrsConstsNotInOxc(workingTsCode, tsConstMap, oxcConstMapInitial)

    // Remove provider consts from both (component metadata, not template output)
    // TS compiler may output "/* unknown */" for complex provider expressions
    let normalizedOxcCode = removeProvidersConst(workingOxcCode)
    normalizedTsCode = removeProvidersConst(normalizedTsCode)

    // Build const value maps AFTER attrs/providers removal
    const oxcConstMap = buildConstValueMap(normalizedOxcCode)
    const normalizedTsConstMap = buildConstValueMap(normalizedTsCode)

    // Create mapping from Oxc const names to TS const names based on value equivalence
    const constMapping = buildConstNameMapping(oxcConstMap, normalizedTsConstMap)

    // Normalize Oxc code by replacing const references to match TS naming
    normalizedOxcCode = replaceConstReferences(normalizedOxcCode, constMapping)

    // Fast path after const normalization
    if (normalizedOxcCode === normalizedTsCode) {
      return { match: true }
    }

    // Parse both with oxc-parser
    const oxcResult = parseSync('oxc.js', normalizedOxcCode, { sourceType: 'module' })
    const tsResult = parseSync('ts.js', normalizedTsCode, { sourceType: 'module' })

    // Check for parse errors
    if (oxcResult.errors.length > 0 || tsResult.errors.length > 0) {
      return {
        match: false,
        diff: [
          {
            path: 'root',
            type: 'different',
            expected: tsResult.errors.length > 0 ? 'parse error' : 'valid',
            actual: oxcResult.errors.length > 0 ? 'parse error' : 'valid',
          },
        ],
      }
    }

    // Normalize both ASTs and compare as strings
    const normalizedOxc = normalizeAst(oxcResult.program)
    const normalizedTs = normalizeAst(tsResult.program)

    if (normalizedOxc === normalizedTs) {
      return { match: true }
    }

    // If they don't match, extract function-level details for reporting
    const oxcFunctions = extractFunctions(
      oxcResult.program as unknown as Program,
      normalizedOxcCode,
    )
    const tsFunctions = extractFunctions(tsResult.program as unknown as Program, normalizedTsCode)

    // Pass constMapping so that const arrow functions (_c0, _c1, etc.) can be matched
    // even when they have different indices but equivalent values
    const functionComparison = compareFunctions(oxcFunctions, tsFunctions, constMapping)

    // IMPORTANT: If all template functions match (no missing, no extra, no diffs),
    // consider it a match even if there are differences in non-function code like consts.
    // This handles cases where:
    // - Const indices differ (_c2 vs _c5) but values/functions are equivalent
    // - Non-template consts (CSS styles, metadata) differ
    const hasNoFunctionDifferences =
      functionComparison.missingFunctions.length === 0 &&
      functionComparison.extraFunctions.length === 0 &&
      functionComparison.functionDiffs.length === 0

    if (hasNoFunctionDifferences) {
      return { match: true, functionComparison }
    }

    return {
      match: false,
      functionComparison,
    }
  } catch (e) {
    return {
      match: false,
      diff: [
        {
          path: 'root',
          type: 'different',
          expected: 'parseable',
          actual: `error: ${e as Error}`,
        },
      ],
    }
  }
}

interface ExtractedFunction {
  ast: FunctionAst
  code: string
  normalized: string
}

/**
 * Extract all named function declarations from a program.
 * Returns a map of function name to its AST, code, and normalized representation.
 *
 * Handles:
 * - FunctionDeclaration: `function foo() { ... }`
 * - VariableDeclaration with FunctionExpression: `const foo = function() { ... }`
 * - VariableDeclaration with ArrowFunctionExpression: `const foo = () => { ... }`
 */
function extractFunctions(program: Program, sourceCode: string): Map<string, ExtractedFunction> {
  const functions = new Map<string, ExtractedFunction>()

  // Pre-compute byte-to-char mapping once for all functions
  const byteToCharMap = createByteToCharMap(sourceCode)

  for (const statement of program.body) {
    // Handle FunctionDeclaration: `function foo() { ... }`
    if (statement.type === 'FunctionDeclaration' && statement.id?.name) {
      const funcAst = statement as FunctionAst
      const code = extractFunctionCode(sourceCode, funcAst, byteToCharMap)
      const normalized = normalizeAst(funcAst)
      functions.set(statement.id.name, { ast: funcAst, code, normalized })
    }

    // Handle VariableDeclaration with function expressions or arrow functions
    // Examples: `const _c0 = function() { ... }` or `const _forTrack = (item) => item.id`
    if (statement.type === 'VariableDeclaration') {
      const varDecl = statement as unknown as VariableDeclaration
      for (const decl of varDecl.declarations) {
        if (
          decl.id?.type === 'Identifier' &&
          decl.id.name &&
          decl.init &&
          (decl.init.type === 'FunctionExpression' || decl.init.type === 'ArrowFunctionExpression')
        ) {
          const varName = decl.id.name
          const funcAst = decl.init as unknown as FunctionAst
          const code = extractFunctionCode(sourceCode, funcAst, byteToCharMap)
          const normalized = normalizeAst(funcAst)
          functions.set(varName, { ast: funcAst, code, normalized })
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
 * Compare functions from both outputs using normalized string comparison.
 *
 * @param oxcFunctions - Functions extracted from Oxc output
 * @param tsFunctions - Functions extracted from TS output
 * @param constMapping - Mapping from Oxc const names to TS const names (based on value equivalence)
 */
function compareFunctions(
  oxcFunctions: Map<string, ExtractedFunction>,
  tsFunctions: Map<string, ExtractedFunction>,
  constMapping?: Map<string, string>,
): FunctionLevelComparison {
  const missingFunctions: string[] = []
  const extraFunctions: string[] = []
  const functionDiffs: FunctionDiff[] = []
  const matchingFunctions: string[] = []

  // Build a mapping from Oxc function names to their corresponding TS names
  // This handles const arrow functions like _c5 -> _c2 when they have the same value
  //
  // IMPORTANT: Only include mappings where both the Oxc name exists in oxcFunctions
  // AND the TS name exists in tsFunctions. This prevents array const mappings
  // (like _c3 -> _c1 for ["animatedContainer"]) from interfering with function matching
  // after code has been normalized via replaceConstReferences.
  const oxcNameToTsName = new Map<string, string>()
  const tsNameToOxcName = new Map<string, string>()

  if (constMapping) {
    for (const [oxcName, tsName] of constMapping) {
      // Only apply mapping for _c{N} const FUNCTIONS (not arrays)
      // Verify that both names actually exist as functions in their respective maps
      if (
        /^_c\d+$/.test(oxcName) &&
        /^_c\d+$/.test(tsName) &&
        oxcFunctions.has(oxcName) &&
        tsFunctions.has(tsName)
      ) {
        oxcNameToTsName.set(oxcName, tsName)
        tsNameToOxcName.set(tsName, oxcName)
      }
    }
  }

  // Find functions that match via the const mapping
  const matchedTsFunctions = new Set<string>()
  const matchedOxcFunctions = new Set<string>()

  // First, match functions by their mapped names
  for (const [oxcName, oxcFunc] of oxcFunctions) {
    const mappedTsName = oxcNameToTsName.get(oxcName) || oxcName
    const tsFunc = tsFunctions.get(mappedTsName)

    if (tsFunc) {
      matchedOxcFunctions.add(oxcName)
      matchedTsFunctions.add(mappedTsName)

      if (oxcFunc.normalized === tsFunc.normalized) {
        matchingFunctions.push(mappedTsName)
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
          name: mappedTsName,
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
  /** Assignment value (normalized) */
  value: string
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

/**
 * Extract static field assignments from code (e.g., ClassName.ɵcmp = ...).
 */
function extractStaticFieldAssignments(code: string): StaticFieldAssignment[] {
  const assignments: StaticFieldAssignment[] = []

  // Match patterns like: ClassName.ɵcmp = ... or ClassName.ɵfac = ...
  // The assignment value extends until the next semicolon (handling nested structures)
  const pattern = /([A-Z][A-Za-z0-9_]*)\.(ɵ[a-z]+)\s*=/g
  let match

  while ((match = pattern.exec(code)) !== null) {
    const className = match[1]
    const fieldName = match[2]
    const assignStart = match.index + match[0].length

    // Find the end of the assignment (matching braces/parens)
    let depth = 0
    let inString: string | null = null
    let escaped = false
    let assignEnd = -1

    for (let i = assignStart; i < code.length; i++) {
      const char = code[i]

      if (escaped) {
        escaped = false
        continue
      }

      if (char === '\\') {
        escaped = true
        continue
      }

      if (inString) {
        if (char === inString) {
          inString = null
        }
        continue
      }

      if (char === '"' || char === "'" || char === '`') {
        inString = char
        continue
      }

      if (char === '(' || char === '{' || char === '[') {
        depth++
      } else if (char === ')' || char === '}' || char === ']') {
        depth--
      } else if (char === ';' && depth === 0) {
        assignEnd = i
        break
      }
    }

    if (assignEnd > assignStart) {
      const value = code.slice(assignStart, assignEnd).trim()
      // Normalize the value (remove whitespace variations)
      const normalizedValue = value.replace(/\s+/g, ' ').replace(/,\s+/g, ',')
      assignments.push({ className, fieldName, value: normalizedValue })
    }
  }

  return assignments
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
}

/**
 * Find matching closing bracket, handling nested structures and strings.
 *
 * @param code - The source code
 * @param startIdx - Index where to start looking (should be at or before the opening bracket)
 * @param open - Opening bracket character ('[', '{', or '(')
 * @param close - Closing bracket character (']', '}', or ')')
 * @returns Index of the matching closing bracket, or -1 if not found
 */
function findMatchingBracket(code: string, startIdx: number, open: string, close: string): number {
  let depth = 0
  let inString: string | null = null

  for (let i = startIdx; i < code.length; i++) {
    const char = code[i]

    if (inString) {
      if (char === inString && code[i - 1] !== '\\') {
        inString = null
      }
      continue
    }

    if (char === '"' || char === "'" || char === '`') {
      inString = char
      continue
    }

    if (char === open) depth++
    else if (char === close) {
      depth--
      if (depth === 0) return i
    }
  }

  return -1
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
function extractClassMetadataCalls(code: string): ClassMetadataInfo[] {
  const results: ClassMetadataInfo[] = []

  // Pattern matches: i0.ɵɵsetClassMetadata(ClassName,
  // Need to handle nested brackets and multiline content
  const startPattern = /i\d+\.ɵɵsetClassMetadata\(\s*(\w+)\s*,\s*/g
  let match

  while ((match = startPattern.exec(code)) !== null) {
    const className = match[1]
    const startIdx = match.index + match[0].length

    // Extract the decorators array (first parameter after className)
    const decoratorsEnd = findMatchingBracket(code, startIdx, '[', ']')
    if (decoratorsEnd === -1) continue
    const decorators = code.slice(startIdx, decoratorsEnd + 1)

    // Skip comma and whitespace to get to ctorParams
    let idx = decoratorsEnd + 1
    while (idx < code.length && (code[idx] === ',' || code[idx] === ' ' || code[idx] === '\n'))
      idx++

    let ctorParams: string | null = null
    if (code[idx] === '[') {
      const ctorEnd = findMatchingBracket(code, idx, '[', ']')
      if (ctorEnd !== -1) {
        ctorParams = code.slice(idx, ctorEnd + 1)
        idx = ctorEnd + 1
      }
    } else if (code.slice(idx, idx + 4) === 'null') {
      ctorParams = null
      idx += 4
    } else if (code.slice(idx, idx + 4) === 'void') {
      // Handle "void 0" which is equivalent to undefined/null
      ctorParams = null
      idx += 6 // Skip "void 0"
    }

    // Skip comma and whitespace to get to propDecorators
    while (idx < code.length && (code[idx] === ',' || code[idx] === ' ' || code[idx] === '\n'))
      idx++

    let propDecorators: string | null = null
    if (code[idx] === '{') {
      const propEnd = findMatchingBracket(code, idx, '{', '}')
      if (propEnd !== -1) {
        propDecorators = code.slice(idx, propEnd + 1)
      }
    } else if (code.slice(idx, idx + 4) === 'null') {
      propDecorators = null
    } else if (code.slice(idx, idx + 4) === 'void') {
      // Handle "void 0" which is equivalent to undefined/null
      propDecorators = null
    }

    results.push({ className, decorators, ctorParams, propDecorators })
  }

  return results
}

/**
 * Normalize a metadata string for comparison.
 * Removes whitespace variations while preserving semantic content.
 */
function normalizeMetadataString(s: string | null): string {
  if (!s) return 'null'
  return s.replace(/\s+/g, ' ').trim()
}

/**
 * Compare class metadata between Oxc and TS outputs.
 *
 * @param oxcMetadata - Class metadata extracted from Oxc output
 * @param tsMetadata - Class metadata extracted from TS output
 * @returns Array of differences found
 */
function compareClassMetadata(
  oxcMetadata: ClassMetadataInfo[],
  tsMetadata: ClassMetadataInfo[],
): ClassMetadataDiff[] {
  const diffs: ClassMetadataDiff[] = []
  const tsMap = new Map(tsMetadata.map((m) => [m.className, m]))
  const oxcMap = new Map(oxcMetadata.map((m) => [m.className, m]))

  // Check for missing/different metadata (in TS but not matching in Oxc)
  for (const [className, tsInfo] of tsMap) {
    const oxcInfo = oxcMap.get(className)
    if (!oxcInfo) {
      diffs.push({
        type: 'missing',
        className,
        field: 'setClassMetadata',
        expected: `decorators: ${tsInfo.decorators.slice(0, 100)}...`,
      })
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

  // Check for extra metadata (in Oxc but not in TS)
  for (const className of oxcMap.keys()) {
    if (!tsMap.has(className)) {
      diffs.push({
        type: 'extra',
        className,
        field: 'setClassMetadata',
        actual: 'found in Oxc output but not in TS',
      })
    }
  }

  return diffs
}

/**
 * Input metadata extracted from defineComponent's inputs object.
 */
interface InputMetadata {
  /** Input index in the inputs array */
  index: number
  /** Binding property name (may differ from public name via alias) */
  bindingName: string
  /** Whether this input has a transform function */
  hasTransform: boolean
  /** Whether this input is required */
  required: boolean
  /** Whether this is a signal-based input */
  isSignal: boolean
}

/**
 * Extract inputs metadata from defineComponent's inputs object.
 *
 * Input format in compiled output:
 * ```javascript
 * inputs: {
 *   "publicName": [inputIndex, "bindingName"],
 *   "publicName2": [inputIndex, "bindingName", transformFn],
 *   "publicName3": [inputIndex, "bindingName", transformFn, required, isSignal],
 * }
 * ```
 */
function extractInputsFromDefineComponent(inputsObject: string): Map<string, InputMetadata> {
  const inputs = new Map<string, InputMetadata>()
  // Match patterns like: "name": [0, "name"] or "name": [1, "name", transformFn]
  const pattern = /"([^"]+)"\s*:\s*\[([^\]]+)\]/g
  let match
  while ((match = pattern.exec(inputsObject)) !== null) {
    const publicName = match[1]
    const valueArray = match[2].split(',').map((s) => s.trim())
    inputs.set(publicName, {
      index: parseInt(valueArray[0], 10),
      bindingName: valueArray[1]?.replace(/"/g, '') || publicName,
      hasTransform:
        valueArray[2] !== undefined && valueArray[2] !== 'undefined' && valueArray[2] !== 'null',
      required: valueArray[3] === 'true',
      isSignal: valueArray[4] === 'true',
    })
  }
  return inputs
}

/**
 * Extract outputs metadata from defineComponent's outputs object.
 *
 * Output format in compiled output:
 * ```javascript
 * outputs: {
 *   "publicName": "internalName",
 *   "aliasedName": "propertyName"
 * }
 * ```
 */
function extractOutputsFromDefineComponent(outputsObject: string): Map<string, string> {
  const outputs = new Map<string, string>()
  // Match patterns like: "clicked": "clicked" or "valueChanged": "changed"
  const pattern = /"([^"]+)"\s*:\s*"([^"]+)"/g
  let match
  while ((match = pattern.exec(outputsObject)) !== null) {
    outputs.set(match[1], match[2])
  }
  return outputs
}

/**
 * Extract inputs and outputs from the ɵcmp defineComponent value.
 *
 * Parses the defineComponent({...}) call to extract the inputs and outputs
 * objects for comparison.
 */
function extractInputsOutputsFromCmpDefinition(cmpValue: string): {
  inputs?: Map<string, InputMetadata>
  outputs?: Map<string, string>
} {
  const result: { inputs?: Map<string, InputMetadata>; outputs?: Map<string, string> } = {}

  // Extract inputs object - match inputs: { ... }
  const inputsMatch = /inputs\s*:\s*\{([^}]*)\}/s.exec(cmpValue)
  if (inputsMatch) {
    result.inputs = extractInputsFromDefineComponent(inputsMatch[1])
  }

  // Extract outputs object - match outputs: { ... }
  const outputsMatch = /outputs\s*:\s*\{([^}]*)\}/s.exec(cmpValue)
  if (outputsMatch) {
    result.outputs = extractOutputsFromDefineComponent(outputsMatch[1])
  }

  return result
}

/**
 * Compare input metadata between Oxc and TS outputs.
 * Returns differences found in input declarations.
 */
function compareInputMetadata(
  oxcInputs: Map<string, InputMetadata>,
  tsInputs: Map<string, InputMetadata>,
  className: string,
): StaticFieldDiff[] {
  const diffs: StaticFieldDiff[] = []

  // Check inputs in TS that should be in Oxc
  for (const [name, metadata] of tsInputs) {
    const oxcMetadata = oxcInputs.get(name)
    if (!oxcMetadata) {
      diffs.push({
        type: 'missing',
        className,
        fieldName: `ɵcmp.inputs.${name}`,
        expected: JSON.stringify(metadata),
      })
    } else {
      // Compare required flag
      if (oxcMetadata.required !== metadata.required) {
        diffs.push({
          type: 'different',
          className,
          fieldName: `ɵcmp.inputs.${name}.required`,
          expected: String(metadata.required),
          actual: String(oxcMetadata.required),
        })
      }
      // Compare isSignal flag
      if (oxcMetadata.isSignal !== metadata.isSignal) {
        diffs.push({
          type: 'different',
          className,
          fieldName: `ɵcmp.inputs.${name}.isSignal`,
          expected: String(metadata.isSignal),
          actual: String(oxcMetadata.isSignal),
        })
      }
      // Compare binding name
      if (oxcMetadata.bindingName !== metadata.bindingName) {
        diffs.push({
          type: 'different',
          className,
          fieldName: `ɵcmp.inputs.${name}.bindingName`,
          expected: metadata.bindingName,
          actual: oxcMetadata.bindingName,
        })
      }
      // Compare hasTransform flag
      if (oxcMetadata.hasTransform !== metadata.hasTransform) {
        diffs.push({
          type: 'different',
          className,
          fieldName: `ɵcmp.inputs.${name}.hasTransform`,
          expected: String(metadata.hasTransform),
          actual: String(oxcMetadata.hasTransform),
        })
      }
    }
  }

  // Check for extra inputs in Oxc not in TS
  for (const name of oxcInputs.keys()) {
    if (!tsInputs.has(name)) {
      diffs.push({
        type: 'extra',
        className,
        fieldName: `ɵcmp.inputs.${name}`,
        actual: JSON.stringify(oxcInputs.get(name)),
      })
    }
  }

  return diffs
}

/**
 * Compare output metadata between Oxc and TS outputs.
 * Returns differences found in output declarations.
 */
function compareOutputMetadata(
  oxcOutputs: Map<string, string>,
  tsOutputs: Map<string, string>,
  className: string,
): StaticFieldDiff[] {
  const diffs: StaticFieldDiff[] = []

  // Check outputs in TS that should be in Oxc
  for (const [publicName, internalName] of tsOutputs) {
    const oxcInternalName = oxcOutputs.get(publicName)
    if (!oxcInternalName) {
      diffs.push({
        type: 'missing',
        className,
        fieldName: `ɵcmp.outputs.${publicName}`,
        expected: internalName,
      })
    } else if (oxcInternalName !== internalName) {
      diffs.push({
        type: 'different',
        className,
        fieldName: `ɵcmp.outputs.${publicName}`,
        expected: internalName,
        actual: oxcInternalName,
      })
    }
  }

  // Check for extra outputs in Oxc not in TS
  for (const [publicName, internalName] of oxcOutputs) {
    if (!tsOutputs.has(publicName)) {
      diffs.push({
        type: 'extra',
        className,
        fieldName: `ɵcmp.outputs.${publicName}`,
        actual: internalName,
      })
    }
  }

  return diffs
}

/**
 * Compare static field assignments between Oxc and TS outputs.
 */
function compareStaticFields(
  oxcFields: StaticFieldAssignment[],
  tsFields: StaticFieldAssignment[],
  constMapping: Map<string, string>,
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
    } else {
      // Compare values after applying const mapping
      let normalizedOxcValue = oxcField.value
      for (const [from, to] of constMapping) {
        normalizedOxcValue = normalizedOxcValue.replace(new RegExp(`\\b${from}\\b`, 'g'), to)
      }

      // Normalize both values for comparison
      const normalizedTs = tsField.value
        .replace(/\s+/g, ' ')
        .replace(/,\s+/g, ',')
        // Remove trailing commas in objects
        .replaceAll(',}', '}')
        // Remove trailing commas in arrays
        .replaceAll(',]', ']')
      const normalizedOxc = normalizedOxcValue
        .replace(/\s+/g, ' ')
        .replace(/,\s+/g, ',')
        // Remove trailing commas in objects
        .replaceAll(',}', '}')
        // Remove trailing commas in arrays
        .replaceAll(',]', ']')

      if (normalizedOxc !== normalizedTs) {
        // For ɵcmp fields, perform granular input/output comparison
        if (tsField.fieldName === 'ɵcmp') {
          const tsInputsOutputs = extractInputsOutputsFromCmpDefinition(tsField.value)
          const oxcInputsOutputs = extractInputsOutputsFromCmpDefinition(oxcField.value)

          // Compare inputs if present in either
          if (tsInputsOutputs.inputs || oxcInputsOutputs.inputs) {
            const inputDiffs = compareInputMetadata(
              oxcInputsOutputs.inputs || new Map(),
              tsInputsOutputs.inputs || new Map(),
              tsField.className,
            )
            diffs.push(...inputDiffs)
          }

          // Compare outputs if present in either
          if (tsInputsOutputs.outputs || oxcInputsOutputs.outputs) {
            const outputDiffs = compareOutputMetadata(
              oxcInputsOutputs.outputs || new Map(),
              tsInputsOutputs.outputs || new Map(),
              tsField.className,
            )
            diffs.push(...outputDiffs)
          }

          // Still report the overall difference if there are other differences
          // beyond inputs/outputs (e.g., template, selectors, etc.)
          diffs.push({
            type: 'different',
            className: tsField.className,
            fieldName: tsField.fieldName,
            expected: tsField.value,
            actual: oxcField.value,
          })
        } else {
          diffs.push({
            type: 'different',
            className: tsField.className,
            fieldName: tsField.fieldName,
            expected: tsField.value,
            actual: oxcField.value,
          })
        }
      }
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
 * Extract class declarations from the program.
 */
function extractClasses(program: Program): Map<string, ClassInfo> {
  const classes = new Map<string, ClassInfo>()

  for (const statement of program.body) {
    if (statement.type === 'ClassDeclaration' && statement.id?.name) {
      const normalized = normalizeAst(statement)
      classes.set(statement.id.name, { name: statement.id.name, normalized })
    }
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
 * - Template functions and other functions
 *
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
    // Format both with oxfmt to normalize formatting
    const [formattedOxc, formattedTs] = await Promise.all([
      formatCodeForComparison(oxcCode),
      formatCodeForComparison(tsCode),
    ])

    // Fast path after formatting normalization
    if (formattedOxc === formattedTs) {
      return { match: true }
    }

    // Normalize template literals to regular strings (semantically equivalent)
    let workingOxcCode = normalizeTemplateLiterals(formattedOxc)
    let workingTsCode = normalizeTemplateLiterals(formattedTs)

    // Normalize PURE comments (oxfmt adds spaces: /* @__PURE__ */ -> /*@__PURE__*/)
    workingOxcCode = workingOxcCode.replaceAll('/* @__PURE__ */', '/*@__PURE__*/')
    workingTsCode = workingTsCode.replaceAll('/* @__PURE__ */', '/*@__PURE__*/')

    // Remove trailing commas in objects: ",}" -> "}"
    workingOxcCode = workingOxcCode.replaceAll(',}', '}')
    workingTsCode = workingTsCode.replaceAll(',}', '}')

    // Remove trailing commas in arrays: ",]" -> "]"
    workingOxcCode = workingOxcCode.replaceAll(',]', ']')
    workingTsCode = workingTsCode.replaceAll(',]', ']')

    // Normalize space before closing brace: " }" -> "}"
    workingOxcCode = workingOxcCode.replaceAll(' }', '}')
    workingTsCode = workingTsCode.replaceAll(' }', '}')

    // Normalize @Inject parameter types
    workingOxcCode = normalizeInjectParameterTypes(workingOxcCode)
    workingTsCode = normalizeInjectParameterTypes(workingTsCode)

    // Normalize nullish coalescing (Oxc preserves ??, TS transpiles to ternary)
    // Apply to TS: convert ternary check to ??
    // Apply to Oxc: remove outer parens around ?? expression
    workingTsCode = normalizeNullishCoalescing(workingTsCode)
    workingOxcCode = removeNullishCoalescingParens(workingOxcCode)

    // Normalize numeric literals: 1e3 → 1000 (OXC emits scientific notation)
    workingOxcCode = normalizeNumericLiterals(workingOxcCode)
    workingTsCode = normalizeNumericLiterals(workingTsCode)

    // Normalize inline object literal formatting: collapse multi-line simple objects
    workingOxcCode = collapseInlineObjectLiterals(workingOxcCode)
    workingTsCode = collapseInlineObjectLiterals(workingTsCode)

    // Fast path after template/inject/nullish/format normalization
    if (workingOxcCode === workingTsCode) {
      return { match: true }
    }

    // Build const value maps for both to identify metadata consts
    const tsConstMap = buildConstValueMap(workingTsCode)
    const oxcConstMapInitial = buildConstValueMap(workingOxcCode)

    // Remove attrs/query consts from TS code that are NOT present in Oxc output
    let normalizedTsCode = removeAttrsConstsNotInOxc(workingTsCode, tsConstMap, oxcConstMapInitial)

    // Remove provider consts from both
    let normalizedOxcCode = removeProvidersConst(workingOxcCode)
    normalizedTsCode = removeProvidersConst(normalizedTsCode)

    // Build const value maps after cleanup
    const oxcConstMap = buildConstValueMap(normalizedOxcCode)
    const normalizedTsConstMap = buildConstValueMap(normalizedTsCode)

    // Create mapping from Oxc const names to TS const names based on value equivalence
    const constMapping = buildConstNameMapping(oxcConstMap, normalizedTsConstMap)

    // Normalize Oxc code by replacing const references to match TS naming
    normalizedOxcCode = replaceConstReferences(normalizedOxcCode, constMapping)

    // Fast path after const normalization
    if (normalizedOxcCode === normalizedTsCode) {
      return { match: true }
    }

    // Parse both with oxc-parser using enhanced options
    const oxcResult = parseSync('oxc.js', normalizedOxcCode, {
      sourceType: 'module',
      showSemanticErrors: true,
    })
    const tsResult = parseSync('ts.js', normalizedTsCode, {
      sourceType: 'module',
      showSemanticErrors: true,
    })

    // Check for parse errors
    const parseErrors: string[] = []
    if (oxcResult.errors.length > 0) {
      parseErrors.push(...oxcResult.errors.map((e) => `Oxc: ${e.message}`))
    }
    if (tsResult.errors.length > 0) {
      parseErrors.push(...tsResult.errors.map((e) => `TS: ${e.message}`))
    }

    if (parseErrors.length > 0) {
      return {
        match: false,
        parseErrors,
      }
    }

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

    // Extract and compare static field assignments
    const oxcFields = extractStaticFieldAssignments(normalizedOxcCode)
    const tsFields = extractStaticFieldAssignments(normalizedTsCode)
    const staticFieldDiffs = compareStaticFields(oxcFields, tsFields, constMapping)

    // Extract and compare class metadata (setClassMetadata calls)
    const oxcClassMetadata = extractClassMetadataCalls(normalizedOxcCode)
    const tsClassMetadata = extractClassMetadataCalls(normalizedTsCode)
    const classMetadataDiffs = compareClassMetadata(oxcClassMetadata, tsClassMetadata)

    // Extract and compare functions (template functions, etc.)
    const oxcFunctions = extractFunctions(
      oxcResult.program as unknown as Program,
      normalizedOxcCode,
    )
    const tsFunctions = extractFunctions(tsResult.program as unknown as Program, normalizedTsCode)
    // Pass constMapping so that const arrow functions (_c0, _c1, etc.) can be matched
    const functionComparison = compareFunctions(oxcFunctions, tsFunctions, constMapping)

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

/**
 * Normalize numeric scientific notation to decimal form using oxc-parser.
 * Finds Literal nodes (ESTree format) whose source text contains scientific notation
 * (e.g. `1e3`) and replaces them with their decimal representation (`1000`).
 */
function normalizeNumericLiterals(code: string): string {
  let ast
  try {
    ast = parseSync('numeric.js', code, { sourceType: 'module' })
  } catch {
    return code
  }

  const replacements: Array<{ start: number; end: number; replacement: string }> = []

  // oxc-parser returns character offsets (not byte offsets), so use start/end directly
  walkAst(ast.program, (node) => {
    if (node.type === 'Literal' && typeof node.value === 'number') {
      if (typeof node.start !== 'number' || typeof node.end !== 'number') return
      const originalText = code.slice(node.start as number, node.end as number)
      // Check if the source text uses scientific notation (exclude hex/octal/binary prefixes)
      if (/e\+?\d/i.test(originalText) && !/^0[xob]/i.test(originalText)) {
        const num = node.value as number
        if (Number.isFinite(num) && Number.isSafeInteger(num)) {
          replacements.push({
            start: node.start as number,
            end: node.end as number,
            replacement: String(num),
          })
        }
      }
    }
  })

  if (replacements.length === 0) return code

  // Apply replacements from end to start
  replacements.sort((a, b) => b.start - a.start)
  let result = code
  for (const { start, end, replacement } of replacements) {
    result = result.slice(0, start) + replacement + result.slice(end)
  }
  return result
}

/**
 * Collapse multi-line simple object literals in function call arguments to single lines.
 *
 * Matches patterns like:
 *   .emit({
 *     content: $event,
 *     format: item_r3.format,
 *     type: item_r3.id,
 *   })
 *
 * And collapses to:
 *   .emit({ content: $event, format: item_r3.format, type: item_r3.id})
 *
 * Only targets objects where every property value is a simple expression
 * (identifiers, member access, $event) — no strings, nested objects, or calls.
 */
const COLLAPSE_OBJ_RE = /\(\{\s*\n((?:\s*\w+:\s*[\w.$]+,?\s*\n)+)\s*\}\)/g

function collapseInlineObjectLiterals(code: string): string {
  return code.replace(COLLAPSE_OBJ_RE, (_match, propsBlock: string) => {
    const lines = propsBlock.trim().split('\n')
    const props = lines.map((l) => l.trim()).filter(Boolean)
    // Remove trailing comma from last property
    const last = props.length - 1
    if (last >= 0 && props[last].endsWith(',')) {
      props[last] = props[last].slice(0, -1)
    }
    return '({ ' + props.join(' ') + '})'
  })
}

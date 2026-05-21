/**
 * Helpers for locating inline `@Component` decorator fields in source text.
 *
 * Regex-based extraction is unreliable here because the field bodies can
 * contain the closing delimiter we'd otherwise rely on — for example, a
 * styles array body commonly contains `]` characters inside attribute
 * selectors (`[data-test="foo"]`), and template strings can contain escaped
 * quotes or backticks. These helpers walk the source character by character,
 * tracking string/template-literal boundaries (including `${…}`
 * interpolations), JavaScript comments (`//` and `/* … *\/`), and the
 * @Component object-literal nesting depth so delimiters inside literals or
 * comments don't affect the search.
 *
 * Known limitations (not handled, fall through to safe defaults):
 *   - **Regex literals** inside `@Component(...)` args. The walker can't
 *     distinguish `/` as a division operator from `/` as a regex-literal
 *     opener without a real JS lexer. Regex literals inside @Component
 *     args don't appear in real Angular code, so this is accepted.
 *   - **Aliased decorator imports**: `@core.Component(...)` or
 *     `import { Component as C }` followed by `@C({...})`. Only the
 *     literal `@Component` form is recognized.
 *   - **Parenthesized decorator expressions** like `@(Component as any)(...)`
 *     — uncommon and not supported.
 *   - **Quoted (`{ 'styles': [...] }`) or computed (`{ ['styles']: [...] }`)
 *     property keys** for the `styles`/`template` field. Almost never used
 *     in Angular; locator returns null for such forms.
 *   - **Concatenated style strings** inside an array (`styles: ['a' + 'b']`)
 *     are extracted as two separate elements; cosmetic but harmless because
 *     the browser sees the same CSS either way.
 *   - **Anonymous default-exported components** (`@Component({...}) export
 *     default class {}`) can't be HMR-addressed (no className) and are
 *     skipped by `locateComponentDecorators`.
 */

// -----------------------------------------------------------------
// Module-level constants & types
// -----------------------------------------------------------------

type Ctx = 'paren' | 'array' | 'brace' | 'sq' | 'dq' | 'tpl'

const OPEN_TO_CTX: Record<string, Ctx> = {
  '(': 'paren',
  '[': 'array',
  '{': 'brace',
  "'": 'sq',
  '"': 'dq',
  '`': 'tpl',
}

const CLOSER_TO_CTX: Record<string, Ctx> = {
  ')': 'paren',
  ']': 'array',
  '}': 'brace',
}

/** JavaScript whitespace, including line terminators and form feeds. */
const WS_RE = /\s/

/** ASCII word characters (JS identifier continuation, minus Unicode). Used
 *  for word-boundary checks around the ASCII field keys `styles`/`template`. */
const ASCII_WORD_RE = /[A-Za-z0-9_$]/

/** Unicode-aware JS identifier characters. Class names can be non-ASCII. */
const IDENT_START_RE = /[\p{L}_$]/u
const IDENT_CONT_RE = /[\p{L}\p{N}_$]/u

/** Opener chars accepted as the value of `styles:` — `string | string[]`. */
const STYLES_OPENERS = '\'"`['
/** Opener chars accepted as the value of `template:` — just string literals. */
const TEMPLATE_OPENERS = '\'"`'

/**
 * If `code[i]` starts a `//` line comment or a `/* … *\/` block comment,
 * return the index just past its end. Otherwise return -1. Caller must
 * ensure it's in a code context (not inside a string or template literal).
 *
 * Pragmatic: this doesn't disambiguate `/` from a regex-literal opener, so
 * regex literals inside `@Component(...)` args remain a known limitation.
 * In practice they don't appear there.
 */
function skipComment(code: string, i: number, end: number): number {
  if (code[i] !== '/') return -1
  const next = code[i + 1]
  if (next === '/') {
    let j = i + 2
    while (j < end && code[j] !== '\n') j++
    return j
  }
  if (next === '*') {
    let j = i + 2
    while (j < end - 1 && !(code[j] === '*' && code[j + 1] === '/')) j++
    return Math.min(j + 2, end)
  }
  return -1
}

/**
 * Process one structural token at `code[i]` against the parsing `stack` and
 * return the next index to read. Mutates `stack` as a side-effect — pushes
 * on opener / `${` / quote, pops on closer / matching quote / end of
 * template literal. Inside string and template contexts, only escape
 * sequences and closers are recognized. In code context, line and block
 * comments are skipped. Mismatched closers are ignored (the stack is
 * unchanged); the caller decides what to do based on its own stop
 * condition.
 *
 * The `end` bound is used for comment-skipping so a block comment can't
 * scan past the caller's intended boundary.
 */
function advanceOneToken(code: string, i: number, stack: Ctx[], end: number): number {
  const top = stack[stack.length - 1]
  const ch = code[i]

  if (top === 'sq' || top === 'dq') {
    if (ch === '\\') return i + 2
    if ((top === 'sq' && ch === "'") || (top === 'dq' && ch === '"')) {
      stack.pop()
    }
    return i + 1
  }

  if (top === 'tpl') {
    if (ch === '\\') return i + 2
    if (ch === '`') {
      stack.pop()
      return i + 1
    }
    if (ch === '$' && code[i + 1] === '{') {
      stack.push('brace')
      return i + 2
    }
    return i + 1
  }

  // Code context. Try a comment first (opaque skip), then a delimiter
  // (string opener, structural opener, or matching closer).
  const afterComment = skipComment(code, i, end)
  if (afterComment !== -1) return afterComment

  const opener = OPEN_TO_CTX[ch]
  if (opener) {
    stack.push(opener)
    return i + 1
  }

  const closerCtx = CLOSER_TO_CTX[ch]
  if (closerCtx && top === closerCtx) {
    stack.pop()
  }
  return i + 1
}

/**
 * Given the index of an opening delimiter (`(`, `[`, `{`, `'`, `"`, or `` ` ``),
 * return the index of its matching closer. Honors string literals, escape
 * sequences, and `${…}` interpolations inside template literals. Returns -1
 * if no balanced closer is found before EOF, or if the character at
 * `openIdx` is not a known delimiter.
 */
export function findClosingDelim(code: string, openIdx: number): number {
  const initial = OPEN_TO_CTX[code[openIdx]]
  if (!initial) return -1

  const stack: Ctx[] = [initial]
  let i = openIdx + 1
  while (i < code.length && stack.length > 0) {
    const next = advanceOneToken(code, i, stack, code.length)
    if (stack.length === 0) {
      // `advanceOneToken` consumed the closer; its index is `next - 1`.
      return next - 1
    }
    i = next
  }
  return -1
}

/**
 * Replace everything between (but not including) the opener/closer at the
 * given inclusive `[start, end]` range with nothing — leaving the original
 * delimiters in place. Works uniformly for `[…]`, `'…'`, `"…"`, and `` `…` ``.
 */
export function emptyDelimitedRange(code: string, range: [number, number]): string {
  const [start, end] = range
  return code.slice(0, start + 1) + code.slice(end)
}

/**
 * Locate a top-level `<field>: <opener>` property inside a `@Component(...)`
 * argument list, returning the inclusive `[start, end]` of the value's outer
 * delimiters. "Top level" means a direct property of the @Component arg
 * object — `{ <field>: ... }` — not a nested object, not inside a string or
 * template literal, not inside a `${...}` interpolation, and not inside a
 * call argument that happens to be a nested object literal.
 *
 * Walks the args body character-by-character tracking string / template /
 * paren / brace / array context, mirroring `findClosingDelim`. A field key
 * is considered only when:
 *   - the parser is at the @Component's immediate object-literal depth
 *     (stack === ['paren', 'brace']);
 *   - the character before `field` is not a word character (word boundary);
 *   - `field` is followed by optional whitespace, then `:`, optional whitespace,
 *     then one of `openerChars`.
 *
 * Returns null if no qualifying field is found.
 */
function locateFieldInsideArgs(
  code: string,
  argsRange: [number, number],
  field: string,
  openerChars: string,
): [number, number] | null {
  const [openParen, closeParen] = argsRange
  const stack: Ctx[] = ['paren']
  let i = openParen + 1

  while (i < closeParen) {
    // Check for a field-key match BEFORE advancing. The match is only
    // valid at the @Component's immediate object-literal depth
    // (`['paren', 'brace']`) — anything deeper is a nested literal that
    // isn't the component's metadata.
    if (
      stack.length === 2 &&
      stack[1] === 'brace' &&
      isFieldKeyAt(code, i, field, closeParen)
    ) {
      let j = i + field.length
      while (j < closeParen && WS_RE.test(code[j])) j++
      if (code[j] === ':') {
        j++
        while (j < closeParen && WS_RE.test(code[j])) j++
        if (j < closeParen && openerChars.includes(code[j])) {
          const end = findClosingDelim(code, j)
          if (end !== -1 && end < closeParen) return [j, end]
        }
      }
    }
    i = advanceOneToken(code, i, stack, closeParen)
  }
  return null
}

/**
 * Whether `field` starts at `position` in `code` AND is bounded on both sides
 * by non-identifier characters (so `template` doesn't match the start of
 * `templateUrl`, and `someStyles:` doesn't match the end of `styles:`).
 */
function isFieldKeyAt(code: string, position: number, field: string, limit: number): boolean {
  if (position + field.length > limit) return false
  if (!code.startsWith(field, position)) return false
  const prev = position > 0 ? code[position - 1] : ''
  if (prev && ASCII_WORD_RE.test(prev)) return false
  const next = code[position + field.length]
  if (next !== undefined && ASCII_WORD_RE.test(next)) return false
  return true
}

/**
 * One @Component decorator paired with the class it decorates.
 */
export interface ComponentDecorator {
  /** Inclusive offsets of the outer `(` and `)` of `@Component(...)`. */
  argsRange: [number, number]
  /** The class name declared after this decorator. */
  className: string
}

/**
 * Enumerate every `@Component(...)` decorator in `code`, pairing each with
 * the class declared immediately after it. Decorators that don't pair to a
 * class (dangling, malformed, anonymous) are skipped — the caller sees only
 * well-formed component declarations.
 *
 * The class-name scan is bounded between this decorator's closing `)` and
 * either the next `@Component\s*\(` or end of file. That bound prevents one
 * decorator's scan from accidentally consuming a sibling's class identifier,
 * and combined with the comment- and string-aware walkers in
 * `findClosingDelim` and `findClassName` it correctly handles `@Component`
 * occurrences inside comments, strings, and template literals.
 *
 * See the module-level docstring for a full list of known limitations.
 */
export function locateComponentDecorators(code: string): ComponentDecorator[] {
  // Pass 1: find every `@Component(...)` and bound its args list.
  type Found = { decoratorStart: number; openParen: number; closeParen: number }
  const decoratorRe = /@Component\s*\(/g
  const found: Found[] = []
  let m: RegExpExecArray | null
  while ((m = decoratorRe.exec(code)) !== null) {
    const openParen = m.index + m[0].length - 1
    const closeParen = findClosingDelim(code, openParen)
    if (closeParen === -1) continue
    found.push({ decoratorStart: m.index, openParen, closeParen })
  }

  // Pass 2: for each decorator, scan forward from its `)` to either the next
  // decorator's `@` or EOF, looking for `class IDENT`. The bound stops one
  // decorator's class-name scan from claiming a sibling's class.
  const out: ComponentDecorator[] = []
  for (let i = 0; i < found.length; i++) {
    const { openParen, closeParen } = found[i]
    const scanEnd = i + 1 < found.length ? found[i + 1].decoratorStart : code.length
    const className = findClassName(code, closeParen + 1, scanEnd)
    if (className !== null) {
      out.push({ argsRange: [openParen, closeParen], className })
    }
  }
  return out
}

/**
 * Find the first `class IDENT` whose `class` keyword appears in
 * `[start, end)`. Returns the identifier (Unicode-aware), or null if no
 * match. Skips line/block comments and string/template literals so a
 * `// uses a base class Bar` comment or a `'class Baz'` string between the
 * decorator and the class declaration can't fool the matcher.
 *
 * Modifiers like `export`, `default`, `abstract`, and additional decorators
 * (`@Foo()`) between the @Component(...) and the class are skipped
 * implicitly — they don't match the `class IDENT` pattern.
 */
function findClassName(code: string, start: number, end: number): string | null {
  let i = start
  while (i < end) {
    // Skip comments (line and block) opaquely.
    const afterComment = skipComment(code, i, end)
    if (afterComment !== -1) {
      i = afterComment
      continue
    }

    // Skip string / template literals opaquely — `class IDENT` text inside
    // them is not a real class declaration. `findClosingDelim` handles
    // escape sequences and `${...}` interpolations correctly.
    const ch = code[i]
    if (ch === "'" || ch === '"' || ch === '`') {
      const close = findClosingDelim(code, i)
      i = close === -1 ? end : close + 1
      continue
    }

    // Try matching the `class` keyword at this position, gated by a word
    // boundary before and after to avoid `subclass`, `classes`, etc.
    if (code.startsWith('class', i)) {
      const prev = i > 0 ? code[i - 1] : ''
      const afterKw = code[i + 5] ?? ''
      if (
        (prev === '' || !IDENT_CONT_RE.test(prev)) &&
        (afterKw === '' || !IDENT_CONT_RE.test(afterKw))
      ) {
        let j = i + 5
        while (j < end && WS_RE.test(code[j])) j++
        if (j < end && IDENT_START_RE.test(code[j])) {
          const idStart = j
          j++
          while (j < end && IDENT_CONT_RE.test(code[j])) j++
          return code.slice(idStart, j)
        }
      }
    }

    i++
  }
  return null
}

/**
 * Locate the `styles:` field inside a specific `@Component(...)` decorator
 * identified by its `argsRange`. Use this when you already have a
 * `ComponentDecorator` in hand (e.g. while iterating
 * `locateComponentDecorators(code)`); it avoids re-enumerating decorators
 * on every lookup, which is the difference between O(N) and O(N²) for
 * files with N components.
 *
 * Returns the inclusive `[start, end]` of the value's outer delimiters,
 * or null if the decorator has no `styles:` field.
 */
export function locateStylesInArgs(
  code: string,
  argsRange: [number, number],
): [number, number] | null {
  return locateFieldInsideArgs(code, argsRange, 'styles', STYLES_OPENERS)
}

/**
 * Locate the `template:` string field inside a specific `@Component(...)`
 * decorator identified by its `argsRange`. See `locateStylesInArgs` for
 * when to prefer this over the className-based variant.
 */
export function locateTemplateInArgs(
  code: string,
  argsRange: [number, number],
): [number, number] | null {
  return locateFieldInsideArgs(code, argsRange, 'template', TEMPLATE_OPENERS)
}

/**
 * Locate the `styles:` field inside the `@Component(...)` decorator that
 * decorates the class named `className`. Convenience wrapper that finds
 * the decorator by className and delegates to `locateStylesInArgs`. The
 * styles value can be an array literal (`[…]`) or a bare string (`'…'`,
 * `"…"`, `` `…` ``) — Angular's `styles` is typed `string | string[]`.
 */
export function locateStylesFieldFor(
  code: string,
  className: string,
): [number, number] | null {
  const found = locateComponentDecorators(code).find((d) => d.className === className)
  return found ? locateStylesInArgs(code, found.argsRange) : null
}

/**
 * Locate the `template:` string field inside the `@Component(...)` decorator
 * that decorates the class named `className`. Convenience wrapper that
 * finds the decorator by className and delegates to `locateTemplateInArgs`.
 */
export function locateTemplateStringFor(
  code: string,
  className: string,
): [number, number] | null {
  const found = locateComponentDecorators(code).find((d) => d.className === className)
  return found ? locateTemplateInArgs(code, found.argsRange) : null
}

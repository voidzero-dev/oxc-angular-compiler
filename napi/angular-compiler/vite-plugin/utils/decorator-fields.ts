/**
 * Helpers for locating inline `@Component` decorator fields in source text.
 *
 * These back the metadata *strip* that decides HMR versus full reload:
 * emptying every `@Component`'s `template:` and `styles:` field and checking
 * the result for byte equality across a save. Per-class resource resolution
 * comes from the Rust extractor, not from here.
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
 *   - **Computed property keys** (`{ ['styles']: [...] }`) can't be resolved
 *     to a name here and so never match a field. Quoted keys are matched,
 *     including ones written with a decodable escape
 *     (`{ 'style\u0073': [...] }`); a key whose escapes cannot be decoded
 *     does not match.
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

/** The only decorator form recognized — see the module docstring. */
const DECORATOR_NAME = 'Component'

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
/**
 * The state of one decorator field, which is three-valued: the key may be
 * missing entirely, present with a value this scan can read, or present with
 * a value it cannot (an identifier, a call, a concatenation).
 *
 * The third state stays apart from "absent" because the two end the walk
 * differently: an unreadable value IS this field, so the search stops there,
 * while an absent key means the search keeps going.
 */
type FieldValue =
  | { kind: 'absent' }
  | { kind: 'unreadable' }
  | { kind: 'literal'; range: [number, number] }

/**
 * Find a top-level field of the `@Component(...)` args and classify its
 * value. See `FieldValue` for the three outcomes.
 *
 * The walk is identical to `locateFieldInsideArgs`, which is expressed on top
 * of this; only the reporting differs.
 */
function findFieldInArgs(
  code: string,
  argsRange: [number, number],
  field: string,
  openerChars: string,
  shorthandMeans: 'absent' | 'unreadable' = 'absent',
): FieldValue {
  const [openParen, closeParen] = argsRange
  const stack: Ctx[] = ['paren']
  let i = openParen + 1
  // Keys sit at the start of the object and after each comma; `:` hands over
  // to the value. Without this, a style-named identifier used as a VALUE
  // (`selector: styleUrl`) would read as a shorthand property.
  let atKey = true

  while (i < closeParen) {
    // A key match is only valid at the @Component's immediate object-literal
    // depth (`['paren', 'brace']`) — anything deeper is a nested literal that
    // isn't the component's metadata.
    if (stack.length === 2 && stack[1] === 'brace') {
      const ch = code[i]
      if (WS_RE.test(ch)) {
        i++
        continue
      }
      const afterComment = skipComment(code, i, closeParen)
      if (afterComment !== -1) {
        i = afterComment
        continue
      }
      if (ch === ',') {
        atKey = true
        i++
        continue
      }
      if (ch === ':') {
        atKey = false
        i++
        continue
      }

      const afterKey = atKey ? matchFieldKeyAt(code, i, field, closeParen) : -1
      if (afterKey !== -1) {
        const j = skipToToken(code, afterKey, closeParen)
        if (code[j] === ':') {
          const v = skipToToken(code, j + 1, closeParen)
          if (v < closeParen && openerChars.includes(code[v])) {
            const end = findClosingDelim(code, v)
            // The literal is the value only when the property ENDS at its
            // closing delimiter. `'<p/>' + SUFFIX` opens with a literal it
            // does not denote, and so do `['.a{}'].concat(MORE)` and
            // `'.a{}' as const`. Returning that leading piece would empty a
            // range the real value extends past, so the strip would no
            // longer be byte-identical for an unrelated edit.
            if (end !== -1 && end < closeParen && endsPropertyValue(code, end + 1, closeParen)) {
              return { kind: 'literal', range: [v, end] }
            }
          }
          // The key is here; its value is not a shape we can read.
          return { kind: 'unreadable' }
        }
        // Shorthand (`{ styles }`): the key stands alone, so the value is
        // a binding this scan cannot follow. Whether that is unknowable or
        // genuinely nothing depends on the field, which is what
        // `shorthandMeans` says. Anything else after the key (`(` for a
        // method or accessor) is not this field at all, so keep scanning.
        if ((code[j] === ',' || code[j] === '}') && shorthandMeans === 'unreadable') {
          return { kind: 'unreadable' }
        }
      }
    }
    i = advanceOneToken(code, i, stack, closeParen)
  }
  return { kind: 'absent' }
}

/**
 * Whether a property value ends at `i`. Inside an object literal a value is
 * closed by `,` or the object's own `}` — or, defensively, the decorator's
 * `)`. Whitespace and comments are skipped first, so a comment may sit
 * between the value and its terminator.
 *
 * Anything else at that position means what was just read is only the LEADING
 * part of a larger expression — a concatenation, a call, a TypeScript
 * assertion — and so is not the value at all. Running out of text counts as
 * ending, since nothing is left to extend the value with.
 */
function endsPropertyValue(code: string, i: number, end: number): boolean {
  const j = skipToToken(code, i, end)
  if (j >= end) return true
  const ch = code[j]
  return ch === ',' || ch === '}' || ch === ')'
}

/** Index of the next character at or after `i` that is not whitespace or a comment. */
function skipToToken(code: string, i: number, end: number): number {
  let j = i
  while (j < end) {
    if (WS_RE.test(code[j])) {
      j++
      continue
    }
    const afterComment = skipComment(code, j, end)
    if (afterComment === -1) break
    j = afterComment
  }
  return j
}

function locateFieldInsideArgs(
  code: string,
  argsRange: [number, number],
  field: string,
  openerChars: string,
): [number, number] | null {
  const found = findFieldInArgs(code, argsRange, field, openerChars)
  return found.kind === 'literal' ? found.range : null
}

/**
 * If a key for `field` starts at `position`, return the index just past it;
 * otherwise -1. Accepts the bare form (`styleUrls:`), the quoted forms
 * (`'styleUrls':`, `"styleUrls":`), and either form written with escapes
 * (`style\u0055rls`, `'style\u0055rls'`) — all of which are valid TS naming
 * the same field, and all of which the Rust extractor resolves.
 *
 * Escapes are decoded rather than refused: decoding keeps the match exact
 * where refusing would silently miss a field that is really there. A key
 * whose escapes cannot be decoded is not matched here, so the field reads
 * as absent.
 *
 * Decoding does not loosen the match: the decoded name must still equal
 * `field` exactly, so `styleUrl` and `styleUrls` stay distinct.
 */
function matchFieldKeyAt(code: string, position: number, field: string, limit: number): number {
  if (isFieldKeyAt(code, position, field, limit)) return position + field.length
  const quote = code[position]
  if (quote === "'" || quote === '"') {
    const close = findClosingDelim(code, position)
    if (close === -1 || close >= limit) return -1
    // A quoted key is a string literal, so it decodes by string rules.
    return decodeEscapes(code.slice(position + 1, close)) === field ? close + 1 : -1
  }
  const id = readIdentifierKey(code, position, limit)
  return id !== null && id.name === field ? id.end : -1
}

/**
 * Read an identifier starting at `position`, decoding escapes, and return the
 * decoded name with the index just past it. Returns null when nothing
 * identifier-like starts there.
 *
 * `name` is null when the identifier carries an escape this scan cannot
 * resolve — malformed hex, a truncated `\u`, a decoded character not valid at
 * its position, or any escape other than `\uHHHH` / `\u{…}`, which are the
 * only two JavaScript permits inside an identifier. The caller must treat
 * that as unknown rather than as a key that isn't there: the Rust extractor
 * parses what this cannot, so "absent" would strip a component's real CSS.
 */
function readIdentifierKey(
  code: string,
  position: number,
  limit: number,
): { name: string | null; end: number } | null {
  let i = position
  let name = ''
  let first = true
  let malformed = false

  while (i < limit) {
    const ch = code[i]

    if (ch === '\\') {
      if (code[i + 1] !== 'u') {
        // `\x41`, `\n`, … are string escapes; in an identifier they are a
        // syntax error, so the name cannot be known from the text.
        malformed = true
        i += 2
        first = false
        continue
      }
      let codePoint = -1
      let next: number
      if (code[i + 2] === '{') {
        const close = code.indexOf('}', i + 3)
        if (close === -1 || close >= limit) return { name: null, end: i + 2 }
        const hex = code.slice(i + 3, close)
        if (HEX_ONLY_RE.test(hex)) codePoint = parseInt(hex, 16)
        next = close + 1
      } else {
        const hex = code.slice(i + 2, i + 6)
        if (hex.length === 4 && HEX_ONLY_RE.test(hex)) codePoint = parseInt(hex, 16)
        next = i + 6
      }
      if (codePoint < 0 || codePoint > 0x10ffff) {
        malformed = true
      } else {
        const decoded = String.fromCodePoint(codePoint)
        if (!(first ? IDENT_START_RE : IDENT_CONT_RE).test(decoded)) malformed = true
        name += decoded
      }
      i = next
      first = false
      continue
    }

    if ((first ? IDENT_START_RE : IDENT_CONT_RE).test(ch)) {
      name += ch
      i++
      first = false
      continue
    }
    break
  }

  if (i === position) return null
  return { name: malformed ? null : name, end: i }
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
 * Enumeration itself skips comments and string/template literals, so a
 * commented-out or quoted `@Component(` is never treated as a decorator.
 * The class-name scan is then bounded between one decorator's closing `)`
 * and the next decorator's `@`, which stops it consuming a sibling's class
 * identifier — a bound that is only correct because phantom occurrences
 * were excluded first.
 *
 * See the module-level docstring for a full list of known limitations.
 */
export function locateComponentDecorators(code: string): ComponentDecorator[] {
  // Pass 1: find every `@Component(...)` and bound its args list. The walk
  // skips comments and string/template literals, so a commented-out or
  // quoted `@Component(` is not mistaken for a real decorator. That matters
  // beyond ignoring it: a phantom occurrence between a real decorator and
  // its class would bound the real one's class-name scan in pass 2, leaving
  // the class paired with the phantom's metadata.
  type Found = { decoratorStart: number; openParen: number; closeParen: number }
  const found: Found[] = []
  let i = 0
  while (i < code.length) {
    const afterComment = skipComment(code, i, code.length)
    if (afterComment !== -1) {
      i = afterComment
      continue
    }
    const ch = code[i]
    if (ch === "'" || ch === '"' || ch === '`') {
      const close = findClosingDelim(code, i)
      i = close === -1 ? code.length : close + 1
      continue
    }
    if (ch === '@' && code.startsWith(DECORATOR_NAME, i + 1)) {
      let j = i + 1 + DECORATOR_NAME.length
      while (j < code.length && WS_RE.test(code[j])) j++
      if (code[j] === '(') {
        const closeParen = findClosingDelim(code, j)
        if (closeParen !== -1) {
          found.push({ decoratorStart: i, openParen: j, closeParen })
          i = closeParen + 1
          continue
        }
      }
    }
    i++
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
 * identified by its `argsRange`, which the caller already holds from
 * iterating `locateComponentDecorators(code)` — so a multi-component file
 * costs one decorator enumeration, not one per lookup.
 *
 * Returns the inclusive `[start, end]` of the value's outer delimiters, or
 * null if the decorator has no `styles:` field. The value may be an array
 * literal (`[…]`) or a bare string (`'…'`, `"…"`, `` `…` ``) — Angular's
 * `styles` is typed `string | string[]`.
 */
export function locateStylesInArgs(
  code: string,
  argsRange: [number, number],
): [number, number] | null {
  return locateFieldInsideArgs(code, argsRange, 'styles', STYLES_OPENERS)
}

/**
 * Locate the `template:` string field inside a specific `@Component(...)`
 * decorator identified by its `argsRange`. Field matching is word-bounded,
 * so a `templateUrl:` field is never read as `template:`.
 */
export function locateTemplateInArgs(
  code: string,
  argsRange: [number, number],
): [number, number] | null {
  return locateFieldInsideArgs(code, argsRange, 'template', TEMPLATE_OPENERS)
}

const SINGLE_CHAR_ESCAPES: Record<string, string> = {
  n: '\n',
  t: '\t',
  r: '\r',
  b: '\b',
  f: '\f',
  v: '\v',
}

/** Line terminators that a backslash may continue across. */
const LINE_TERMINATORS = new Set(['\n', '\r', ' ', ' '])

const HEX_ONLY_RE = /^[0-9a-fA-F]+$/

/**
 * Decode the escape sequences in a string- or template-literal's raw source
 * text into the value it actually denotes, or null if the text holds an
 * escape this cannot resolve.
 *
 * The raw text is what the source spells; the decoded text is what the Rust
 * extractor sees and what a path or a stylesheet must be compared against.
 * `'./a.css'` names `./a.css`, and reading it raw yields a path that
 * does not exist.
 *
 * Null is returned for a malformed escape (`\xZZ`, a truncated `\u12`) and
 * for the legacy octal forms, which are outright errors in a module. Those
 * are cases where guessing a value would be worse than telling the caller
 * this literal is beyond the scan.
 *
 * A raw text with no backslash is returned unchanged, so the overwhelmingly
 * common literal costs nothing and reads byte for byte as before.
 */
function decodeEscapes(raw: string): string | null {
  if (!raw.includes('\\')) return raw

  let out = ''
  let i = 0
  while (i < raw.length) {
    const ch = raw[i]
    if (ch !== '\\') {
      out += ch
      i++
      continue
    }

    const next = raw[i + 1]
    // A trailing backslash cannot occur in a well-delimited literal, since
    // it would have escaped the closing quote.
    if (next === undefined) return null

    // Line continuation: the backslash and the terminator both vanish.
    if (LINE_TERMINATORS.has(next)) {
      i += next === '\r' && raw[i + 2] === '\n' ? 3 : 2
      continue
    }

    const single = SINGLE_CHAR_ESCAPES[next]
    if (single !== undefined) {
      out += single
      i += 2
      continue
    }

    if (next === 'x') {
      const hex = raw.slice(i + 2, i + 4)
      if (hex.length < 2 || !HEX_ONLY_RE.test(hex)) return null
      out += String.fromCharCode(parseInt(hex, 16))
      i += 4
      continue
    }

    if (next === 'u') {
      if (raw[i + 2] === '{') {
        const close = raw.indexOf('}', i + 3)
        if (close === -1) return null
        const hex = raw.slice(i + 3, close)
        if (!HEX_ONLY_RE.test(hex)) return null
        const code = parseInt(hex, 16)
        if (code > 0x10ffff) return null
        out += String.fromCodePoint(code)
        i = close + 1
        continue
      }
      const hex = raw.slice(i + 2, i + 6)
      if (hex.length < 4 || !HEX_ONLY_RE.test(hex)) return null
      out += String.fromCharCode(parseInt(hex, 16))
      i += 6
      continue
    }

    // `\0` is NUL only when no digit follows; with one it is a legacy octal
    // escape, which a module rejects outright. `\1`-`\9` likewise.
    if (next >= '0' && next <= '9') {
      if (next === '0' && !(raw[i + 2] >= '0' && raw[i + 2] <= '9')) {
        out += '\0'
        i += 2
        continue
      }
      return null
    }

    // Anything else denotes itself: `\\`, `\'`, `\"`, `` \` ``, `\$`, and
    // any other non-escape character.
    out += next
    i += 2
  }
  return out
}

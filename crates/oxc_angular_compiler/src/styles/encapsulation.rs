//! CSS style encapsulation for Angular components.
//!
//! This module implements Angular's ViewEncapsulation.Emulated behavior,
//! which scopes CSS styles to a component by adding attribute selectors.
//!
//! This is a port of Angular's ShadowCss class from:
//! `packages/compiler/src/shadow_css.ts`
//!
//! ## How it works
//!
//! For each CSS rule, we add a component-specific attribute selector:
//!
//! ```css
//! /* Input */
//! .button { color: red; }
//! h1, h2 { font-weight: bold; }
//!
//! /* Output (selector = "contenta") */
//! .button[contenta] { color: red; }
//! h1[contenta], h2[contenta] { font-weight: bold; }
//! ```
//!
//! Special handling for:
//! - `:host` selectors → replaced with the host attribute selector
//! - `:host-context()` selectors → context-based scoping
//! - `::ng-deep` → removed (deprecated but still supported)
//! - Media queries, keyframes, etc. → preserved

use std::{fmt::Write as _, ops::Range};

use crate::component::{AngularVersion, Prerelease};

/// Comments are replaced by `%COMMENT<n>%` placeholders during processing,
/// where `<n>` is the comment's index in the extracted list.
///
/// Angular uses a bare `%COMMENT%` here, but that only restores correctly when
/// every placeholder survives exactly once and in order. Neither holds: the
/// `:host-context()` pass duplicates selector text (and any placeholder in it)
/// once per generated permutation, and `polyfill-next-selector` discards text.
/// With an unindexed placeholder each duplicate consumes the *next* comment,
/// so a following `/*# sourceMappingURL=... */` gets teleported into the middle
/// of a selector and a literal `%COMMENT%` is left behind in the shipped CSS.
/// Carrying the index makes restoration independent of both count and order.
const COMMENT_PLACEHOLDER_PREFIX: &str = "%COMMENT";

/// The placeholder standing in for `comments[index]`.
fn comment_placeholder(index: usize) -> String {
    format!("{COMMENT_PLACEHOLDER_PREFIX}{index}%")
}

/// Find the comment placeholder at or after `from`, returning its byte range
/// and the comment index it carries.
fn find_comment_placeholder(s: &str, from: usize) -> Option<(Range<usize>, usize)> {
    let mut at = from;
    while let Some(rel) = s[at..].find(COMMENT_PLACEHOLDER_PREFIX) {
        let start = at + rel;
        let digits_at = start + COMMENT_PLACEHOLDER_PREFIX.len();
        let after = &s[digits_at..];
        let digits = after.bytes().take_while(u8::is_ascii_digit).count();
        if digits > 0
            && after.as_bytes().get(digits) == Some(&b'%')
            && let Ok(index) = after[..digits].parse()
        {
            return Some((start..digits_at + digits + 1, index));
        }
        at = digits_at;
    }
    None
}

/// Remove every comment placeholder from `s`, returning the cleaned text and
/// the removed placeholders concatenated in their original order.
fn strip_comment_placeholders(s: &str) -> (String, String) {
    let mut cleaned = String::with_capacity(s.len());
    let mut removed = String::new();
    let mut at = 0;
    while let Some((range, _)) = find_comment_placeholder(s, at) {
        cleaned.push_str(&s[at..range.start]);
        at = range.end;
        removed.push_str(&s[range]);
    }
    cleaned.push_str(&s[at..]);
    (cleaned, removed)
}

// Polyfill host markers (matching Angular's shadow_css.ts)
const POLYFILL_HOST: &str = "-shadowcsshost";
const POLYFILL_HOST_NO_COMBINATOR: &str = "-shadowcsshost-no-combinator";
const POLYFILL_HOST_CONTEXT: &str = "-shadowcsscontext";

/// Push a single UTF-8 character starting at byte position `i` from `source` into `result`.
/// Returns the number of bytes consumed (1 for ASCII, 2-4 for multi-byte).
///
/// This replaces the incorrect `result.push(bytes[i] as char)` pattern which
/// corrupts multi-byte UTF-8 characters by treating each byte as a Latin-1 codepoint.
#[inline]
fn push_utf8_char(result: &mut String, source: &str, i: usize) -> usize {
    // Determine UTF-8 character width from the leading byte per RFC 3629.
    let b = source.as_bytes()[i];
    let width = if b < 0x80 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    };
    result.push_str(&source[i..i + width]);
    width
}

// =============================================================================
// SafeSelector - Escapes problematic CSS patterns before processing
// =============================================================================

/// Port of Angular's `SafeSelector`: hides attribute selectors, backslash
/// escapes and `:nth-*()` arguments behind placeholders while a selector is
/// split on combinators, so the spaces, `+` and `~` inside them are not taken
/// for combinators.
#[derive(Default)]
struct SafeSelector {
    placeholders: Vec<String>,
    content: String,
}

impl SafeSelector {
    fn new(selector: &str) -> Self {
        let mut placeholders = Vec::new();

        // `/(\[[^\]]*\])/g` -> `__ph-N__`
        let mut escaped = String::with_capacity(selector.len());
        let mut rest = selector;
        while let Some(open) = rest.find('[') {
            let Some(close) = rest[open..].find(']').map(|i| open + i) else {
                break;
            };
            escaped.push_str(&rest[..open]);
            let _ = write!(escaped, "__ph-{}__", placeholders.len());
            placeholders.push(rest[open..=close].to_string());
            rest = &rest[close + 1..];
        }
        escaped.push_str(rest);

        // `/(\\.)/g` -> `__esc-ph-N__` (`.` stops at line terminators)
        let selector = escaped;
        let mut escaped = String::with_capacity(selector.len());
        let mut chars = selector.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\\'
                && let Some(&next) = chars.peek()
                && !is_js_line_terminator(next)
            {
                chars.next();
                let _ = write!(escaped, "__esc-ph-{}__", placeholders.len());
                placeholders.push(format!("\\{next}"));
            } else {
                escaped.push(c);
            }
        }

        // `nthRegex`, `/(:nth-[-\w]+)<_parenSuffix>/g` -> `$1__ph-N__`
        let selector = escaped;
        let bytes = selector.as_bytes();
        let mut content = String::with_capacity(selector.len());
        let mut copied = 0;
        let mut at = 0;
        while let Some(found) = selector[at..].find(":nth-").map(|i| at + i) {
            let name_end = found
                + ":nth-".len()
                + bytes[found + ":nth-".len()..]
                    .iter()
                    .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
                    .count();
            let close = (name_end > found + ":nth-".len())
                .then(|| match_paren_suffix(&selector, name_end))
                .flatten();
            let Some(close) = close else {
                at = found + 1;
                continue;
            };
            content.push_str(&selector[copied..name_end]);
            let _ = write!(content, "__ph-{}__", placeholders.len());
            placeholders.push(selector[name_end..=close].to_string());
            copied = close + 1;
            at = close + 1;
        }
        content.push_str(&selector[copied..]);

        Self { placeholders, content }
    }

    /// `content.replace(/__(?:ph|esc-ph)-(\d+)__/g, (_ph, index) => this.placeholders[+index])`.
    ///
    /// A single pass: a placeholder inside a restored value stays as it is.
    fn restore(&self, content: &str) -> String {
        let mut result = String::with_capacity(content.len());
        let mut copied = 0;
        let mut from = 0;
        while let Some(at) = content[from..].find("__").map(|i| from + i) {
            let rest = &content[at + 2..];
            let body = rest.strip_prefix("ph-").or_else(|| rest.strip_prefix("esc-ph-"));
            let parsed = body.and_then(|body| {
                let digits = body.bytes().take_while(u8::is_ascii_digit).count();
                (digits > 0 && body[digits..].starts_with("__")).then(|| {
                    let end = content.len() - body.len() + digits + 2;
                    (&body[..digits], end)
                })
            });
            let Some((index, end)) = parsed else {
                from = at + 1;
                continue;
            };
            result.push_str(&content[copied..at]);
            // An out-of-range index restores as `undefined`, as in JavaScript.
            match index.parse::<usize>().ok().and_then(|i| self.placeholders.get(i)) {
                Some(placeholder) => result.push_str(placeholder),
                None => result.push_str("undefined"),
            }
            copied = end;
            from = end;
        }
        result.push_str(&content[copied..]);
        result
    }
}

/// Shim CSS text with the given selectors.
///
/// This is the main entry point matching Angular's `ShadowCss.shimCssText()`.
/// It follows the latest Angular release; use [`shim_css_text_for_version`]
/// to match an older one.
///
/// # Arguments
///
/// * `css` - The CSS source code to encapsulate
/// * `content_attr` - The attribute added to all elements inside the host (e.g., `_ngcontent-xxx` or just `contenta` for tests)
/// * `host_attr` - The attribute added to the host itself (optional, defaults to empty)
///
/// # Returns
///
/// The CSS with all selectors scoped to the component.
///
/// # Example
///
/// ```
/// use oxc_angular_compiler::styles::shim_css_text;
///
/// let css = ".button { color: red; }";
/// let result = shim_css_text(css, "contenta", "");
/// assert!(result.contains("[contenta]"));
/// ```
pub fn shim_css_text(css: &str, content_attr: &str, host_attr: &str) -> String {
    shim_css_text_for_version(css, content_attr, host_attr, None)
}

/// Shim CSS text the way the given Angular version's `ShadowCss` does.
///
/// `None` means the latest Angular release. Versions before v22.0.0-rc.0 also
/// run the legacy Shadow DOM polyfill passes (`polyfill-next-selector`,
/// `polyfill-rule`, `polyfill-unscoped-rule`, `::shadow`, `::content`); see
/// [`AngularVersion::uses_shadow_css_polyfills`]. Versions before
/// v22.2.0-next.2 predate the CSS nesting support of `ShadowCss`: they scope
/// a lone `&` and rules without a block, and keep rule bodies as written.
pub fn shim_css_text_for_version(
    css: &str,
    content_attr: &str,
    host_attr: &str,
    angular_version: Option<AngularVersion>,
) -> String {
    if css.is_empty() {
        return String::new();
    }

    let legacy_polyfills = angular_version.is_some_and(|v| v.uses_shadow_css_polyfills());

    // Step 0: Extract comments and replace with placeholders
    // This prevents comment contents from interfering with CSS parsing
    let (mut result, comments) = extract_comments(css);

    // Step 1: Process polyfill directives (polyfill-next-selector, polyfill-rule)
    // These convert ShadowDOM rules to work with the CSS shim
    // Step 2: Extract unscoped rules (polyfill-unscoped-rule)
    // These rules are added at the end WITHOUT scoping
    let mut unscoped_rules = String::new();
    if legacy_polyfills {
        result = insert_polyfill_directives(&result);
        result = insert_polyfill_rules(&result);
        unscoped_rules = extract_unscoped_rules(&result);
        result = remove_unscoped_rules(&result);
    }

    // Step 3: Replace `:host-context(...)` and `:host` with polyfill markers,
    // then expand them like `_scopeCssText`: `:host` first, then
    // `:host-context()`. The markers become attributes during scoping.
    result = insert_polyfill_host(&result);
    result = convert_colon_host(&result);
    result = convert_colon_host_context(&result);

    // Step 5: Handle ::shadow and ::content (removed in v22)
    if legacy_polyfills {
        result = convert_shadow_dom_selectors(&result);
    }

    // Step 6: Scope keyframes and animation properties
    if !content_attr.is_empty() {
        result = scope_keyframes_related_css(&result, content_attr);
    }

    // Step 7: Scope all selectors with the content attribute
    if !content_attr.is_empty() {
        let css_nesting = has_css_nesting_support(angular_version);
        result = scope_selectors(&result, content_attr, host_attr, css_nesting);
    }

    // Step 8: Append unscoped rules (without scoping)
    if !unscoped_rules.is_empty() {
        result = result + "\n" + unscoped_rules.trim();
    }

    // Step 9: Trim, then restore comments. Angular's `_scopeCssText` trims
    // before the comments come back, so newlines kept from a leading or
    // trailing comment survive.
    restore_comments(js_trim(&result), &comments)
}

/// Extract comments and replace them with placeholders.
/// Sourcemap comments are preserved, other comments are replaced with newlines.
fn extract_comments(css: &str) -> (String, Vec<String>) {
    let mut comments = Vec::new();
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for comment start: /*
        if i + 1 < len && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            let comment_start = i;
            i += 2;

            // Find comment end: */
            while i + 1 < len {
                if bytes[i] == b'*' && bytes[i + 1] == b'/' {
                    i += 2;
                    break;
                }
                i += 1;
            }

            let comment = &css[comment_start..i];

            // Check if it's a sourcemap comment: /* # source... or /*# source...
            // Matches regex r"/\*\s*#\s*source"
            let is_sourcemap = {
                let after_start = &comment[2..]; // skip /*
                let trimmed = after_start.trim_start();
                trimmed.starts_with('#') && trimmed[1..].trim_start().starts_with("source")
            };

            let index = comments.len();
            if is_sourcemap {
                comments.push(comment.to_string());
            } else {
                // Preserve only the newlines that were inside the comment. Angular
                // v22 no longer appends an extra trailing newline (the old `+ '\n'`),
                // so removing a comment doesn't insert blank lines.
                let newline_count = comment.bytes().filter(|&b| b == b'\n').count();
                let mut preserved = String::new();
                for _ in 0..newline_count {
                    preserved.push('\n');
                }
                comments.push(preserved);
            }

            result.push_str(&comment_placeholder(index));
        } else {
            i += push_utf8_char(&mut result, css, i);
        }
    }

    (result, comments)
}

/// Restore comments from placeholders.
///
/// Every placeholder carries the index of the comment it stands for, so a
/// placeholder that got duplicated (`:host-context()` permutations) restores to
/// the same comment as its twin, and one that got dropped shifts nothing.
///
/// `extract_comments` only ever emits in-range indices, and the passes that
/// copy selector text copy the index along with it, so every placeholder *we*
/// generated resolves. An index that doesn't is therefore source CSS that
/// merely looks like a placeholder (`content: "%COMMENT7%"`), and is left
/// exactly as the author wrote it.
fn restore_comments(css: &str, comments: &[String]) -> String {
    let mut result = String::with_capacity(css.len());
    let mut at = 0;

    while let Some((range, index)) = find_comment_placeholder(css, at) {
        result.push_str(&css[at..range.start]);
        result.push_str(comments.get(index).map_or(&css[range.start..range.end], String::as_str));
        at = range.end;
    }
    result.push_str(&css[at..]);

    result
}

// =============================================================================
// Keyframe scoping
// =============================================================================

use std::collections::HashSet;

/// Animation keywords that should not be scoped as animation names.
const ANIMATION_KEYWORDS: &[&str] = &[
    // global values
    "inherit",
    "initial",
    "revert",
    "unset",
    // animation-direction
    "alternate",
    "alternate-reverse",
    "normal",
    "reverse",
    // animation-fill-mode
    "backwards",
    "both",
    "forwards",
    "none",
    // animation-play-state
    "paused",
    "running",
    // animation-timing-function
    "ease",
    "ease-in",
    "ease-in-out",
    "ease-out",
    "linear",
    "step-start",
    "step-end",
    // steps() function
    "end",
    "jump-both",
    "jump-end",
    "jump-none",
    "jump-start",
    "start",
];

/// Scope keyframes rules and animation properties.
fn scope_keyframes_related_css(css: &str, scope_selector: &str) -> String {
    // First pass: scope @keyframes names and collect them
    let mut local_keyframes: HashSet<String> = HashSet::new();
    let result = scope_keyframes_names(css, scope_selector, &mut local_keyframes);

    // Second pass: scope animation and animation-name properties
    scope_animation_rules(&result, scope_selector, &local_keyframes)
}

/// Find and replace @keyframes names with scoped versions.
fn scope_keyframes_names(
    css: &str,
    scope_selector: &str,
    local_keyframes: &mut HashSet<String>,
) -> String {
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for @keyframes or @-webkit-keyframes
        if bytes[i] == b'@' {
            let remaining = &css[i..];

            // Try @-webkit-keyframes first (longer)
            let (prefix_end, is_keyframes) =
                if remaining.len() >= 18 && remaining[1..].starts_with("-webkit-keyframes") {
                    (i + 18, true)
                } else if remaining.len() >= 10 && remaining[1..].starts_with("keyframes") {
                    (i + 10, true)
                } else {
                    (i, false)
                };

            if is_keyframes {
                // Skip whitespace after @keyframes
                let mut j = prefix_end;
                while j < len && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }

                if j < len {
                    let prefix = &css[i..j];

                    // Check for quoted or unquoted name
                    let (name, name_end, quote_char) = if bytes[j] == b'\'' {
                        // Single-quoted name
                        let name_start = j + 1;
                        let mut name_end = name_start;
                        while name_end < len && bytes[name_end] != b'\'' {
                            // Handle escaped quotes
                            if bytes[name_end] == b'\\' && name_end + 1 < len {
                                name_end += 2;
                            } else {
                                name_end += 1;
                            }
                        }
                        let name = &css[name_start..name_end];
                        (name, name_end + 1, Some('\''))
                    } else if bytes[j] == b'"' {
                        // Double-quoted name
                        let name_start = j + 1;
                        let mut name_end = name_start;
                        while name_end < len && bytes[name_end] != b'"' {
                            // Handle escaped quotes
                            if bytes[name_end] == b'\\' && name_end + 1 < len {
                                name_end += 2;
                            } else {
                                name_end += 1;
                            }
                        }
                        let name = &css[name_start..name_end];
                        (name, name_end + 1, Some('"'))
                    } else if is_valid_unquoted_keyframe_name_start(bytes[j]) {
                        // Unquoted name: [A-Za-z_][\w-]*
                        let name_start = j;
                        let mut name_end = j + 1;
                        while name_end < len
                            && is_valid_unquoted_keyframe_name_char(bytes[name_end])
                        {
                            name_end += 1;
                        }
                        let name = &css[name_start..name_end];
                        (name, name_end, None)
                    } else {
                        // No valid name found
                        i += push_utf8_char(&mut result, css, i);
                        continue;
                    };

                    // Capture trailing whitespace
                    let mut trailing_end = name_end;
                    while trailing_end < len && bytes[trailing_end].is_ascii_whitespace() {
                        trailing_end += 1;
                    }
                    let trailing = &css[name_end..trailing_end];

                    // Store the unquoted name for animation scoping
                    let unquoted_name = if quote_char.is_some() {
                        unescape_quotes(name, true)
                    } else {
                        name.to_string()
                    };
                    local_keyframes.insert(unquoted_name);

                    // Write the scoped version
                    match quote_char {
                        Some('\'') => {
                            result.push_str(prefix);
                            result.push('\'');
                            result.push_str(scope_selector);
                            result.push('_');
                            result.push_str(name);
                            result.push('\'');
                            result.push_str(trailing);
                        }
                        Some('"') => {
                            result.push_str(prefix);
                            result.push('"');
                            result.push_str(scope_selector);
                            result.push('_');
                            result.push_str(name);
                            result.push('"');
                            result.push_str(trailing);
                        }
                        None => {
                            result.push_str(prefix);
                            result.push_str(scope_selector);
                            result.push('_');
                            result.push_str(name);
                            result.push_str(trailing);
                        }
                        Some(q) => {
                            // Other quote characters (shouldn't happen in practice,
                            // but handle gracefully).
                            result.push_str(prefix);
                            result.push(q);
                            result.push_str(scope_selector);
                            result.push('_');
                            result.push_str(name);
                            result.push(q);
                            result.push_str(trailing);
                        }
                    }

                    i = trailing_end;
                    continue;
                }
            }
        }

        i += push_utf8_char(&mut result, css, i);
    }

    result
}

/// Check if a byte is a valid start character for an unquoted keyframe name.
fn is_valid_unquoted_keyframe_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

/// Check if a byte is a valid continuation character for an unquoted keyframe name.
fn is_valid_unquoted_keyframe_name_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// Unescape quotes in a string if it was quoted.
fn unescape_quotes(s: &str, is_quoted: bool) -> String {
    if !is_quoted {
        return s.to_string();
    }
    // Remove backslash escape sequences before quotes
    // e.g., \' becomes ' and \" becomes "
    s.replace("\\'", "'").replace("\\\"", "\"")
}

/// Scope animation-name and animation property values.
fn scope_animation_rules(
    css: &str,
    scope_selector: &str,
    local_keyframes: &HashSet<String>,
) -> String {
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check if we're at a position where an animation property could start
        // (start of string, after whitespace, or after semicolon)
        let at_property_start = i == 0
            || bytes[i - 1].is_ascii_whitespace()
            || bytes[i - 1] == b';'
            || bytes[i - 1] == b'{';

        if at_property_start && css.is_char_boundary(i) {
            // Try to match animation or animation-name property
            if let Some((prefix_end, value_start, value_end, is_animation_name)) =
                find_animation_property(&css[i..])
            {
                // Ensure all slice boundaries are valid
                if !css.is_char_boundary(i + prefix_end)
                    || !css.is_char_boundary(i + value_start)
                    || !css.is_char_boundary(i + value_end)
                {
                    i += push_utf8_char(&mut result, css, i);
                    continue;
                }
                let prefix = &css[i..i + prefix_end];
                let value = &css[i + value_start..i + value_end];

                result.push_str(prefix);

                if is_animation_name {
                    // animation-name: just scope the names
                    let scoped_names: Vec<String> = value
                        .split(',')
                        .map(|name| scope_animation_keyframe(name, scope_selector, local_keyframes))
                        .collect();
                    result.push_str(&scoped_names.join(","));
                } else {
                    // animation: scope the full declaration
                    let scoped_decl =
                        scope_animation_declaration(value, scope_selector, local_keyframes);
                    result.push_str(&scoped_decl);
                }

                i += value_end;
                continue;
            }
        }

        i += push_utf8_char(&mut result, css, i);
    }

    result
}

/// Find an animation or animation-name property starting at the given position.
/// Returns (prefix_end, value_start, value_end, is_animation_name) if found.
fn find_animation_property(s: &str) -> Option<(usize, usize, usize, bool)> {
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Check for -webkit- prefix
    let prop_start = if s.starts_with("-webkit-") { 8 } else { 0 };

    // Check for animation-name or animation
    let remaining = &s[prop_start..];
    let (prop_name_end, is_animation_name) = if remaining.starts_with("animation-name") {
        (prop_start + 14, true)
    } else if remaining.starts_with("animation") {
        // Make sure it's not animation-* (other than animation-name)
        let after_animation = &remaining[9..];
        if after_animation.starts_with("-") && !after_animation.starts_with("-name") {
            return None;
        }
        (prop_start + 9, false)
    } else {
        return None;
    };

    // Skip whitespace after property name
    let mut i = prop_name_end;
    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    // Expect colon
    if i >= len || bytes[i] != b':' {
        return None;
    }
    i += 1;

    // Skip whitespace after colon
    while i < len && bytes[i].is_ascii_whitespace() {
        i += 1;
    }

    // Skip any leading commas (the regex had `,*`)
    while i < len && bytes[i] == b',' {
        i += 1;
    }

    let value_start = i;

    // Find the end of the value (until semicolon, closing brace, or end)
    while i < len && bytes[i] != b';' && bytes[i] != b'}' {
        i += 1;
    }

    let value_end = i;

    // Don't match empty values
    if value_start >= value_end {
        return None;
    }

    Some((value_start, value_start, value_end, is_animation_name))
}

/// Scope animation property names in a declaration like "foo 10s ease".
fn scope_animation_declaration(
    declaration: &str,
    scope_selector: &str,
    local_keyframes: &HashSet<String>,
) -> String {
    // Parse the declaration and scope keyframe names
    // Split by whitespace, keeping track of what could be an animation name
    let mut result = String::new();
    let mut chars = declaration.chars().peekable();
    let mut current_word = String::new();

    while let Some(c) = chars.next() {
        if c.is_whitespace() || c == ',' {
            // End of a word - process it
            if !current_word.is_empty() {
                result.push_str(&scope_animation_word(
                    &current_word,
                    scope_selector,
                    local_keyframes,
                ));
                current_word.clear();
            }
            result.push(c);
        } else if c == '(' {
            // Start of a function - skip until matching close paren
            result.push_str(&current_word);
            current_word.clear();
            result.push(c);
            let mut depth = 1;
            while depth > 0 {
                if let Some(next) = chars.next() {
                    result.push(next);
                    if next == '(' {
                        depth += 1;
                    } else if next == ')' {
                        depth -= 1;
                    }
                } else {
                    break;
                }
            }
        } else {
            current_word.push(c);
        }
    }

    // Handle the last word
    if !current_word.is_empty() {
        result.push_str(&scope_animation_word(&current_word, scope_selector, local_keyframes));
    }

    result
}

/// Scope a single word from an animation declaration if it's a keyframe name.
fn scope_animation_word(
    word: &str,
    scope_selector: &str,
    local_keyframes: &HashSet<String>,
) -> String {
    // Check if it's a valid CSS identifier that could be a keyframe name
    let is_valid_ident =
        word.chars().next().map_or(false, |c| c.is_alphabetic() || c == '-' || c == '_')
            && word.chars().all(|c| c.is_alphanumeric() || c == '-' || c == '_');

    if !is_valid_ident {
        return word.to_string();
    }

    // Don't scope animation keywords
    if ANIMATION_KEYWORDS.contains(&word) {
        return word.to_string();
    }

    // Scope if it's a local keyframe
    if local_keyframes.contains(word) {
        format!("{}_{}", scope_selector, word)
    } else {
        word.to_string()
    }
}

/// Scope a single animation keyframe name (preserving surrounding whitespace and quotes).
fn scope_animation_keyframe(
    keyframe: &str,
    scope_selector: &str,
    local_keyframes: &HashSet<String>,
) -> String {
    let trimmed = keyframe.trim();
    if trimmed.is_empty() {
        return keyframe.to_string();
    }

    // Preserve leading/trailing whitespace
    let leading = &keyframe[..keyframe.len() - keyframe.trim_start().len()];
    let trailing = &keyframe[keyframe.trim_end().len()..];

    // Check for quotes
    let (quote, name) = if (trimmed.starts_with('"') && trimmed.ends_with('"'))
        || (trimmed.starts_with('\'') && trimmed.ends_with('\''))
    {
        let q = &trimmed[..1];
        let n = &trimmed[1..trimmed.len() - 1];
        (q, n)
    } else {
        ("", trimmed)
    };

    let unescaped = unescape_quotes(name, !quote.is_empty());
    let scoped_name = if local_keyframes.contains(&unescaped) {
        format!("{}_{}", scope_selector, name)
    } else {
        name.to_string()
    };

    format!("{}{}{}{}{}", leading, quote, scoped_name, quote, trailing)
}

/// Backwards-compatible function that generates the attribute format internally.
///
/// This is the original API that generates `[ng-c{id}]` format.
/// Consider using `shim_css_text` directly for more control.
pub fn encapsulate_style(css: &str, component_id: &str) -> String {
    encapsulate_style_for_version(css, component_id, None)
}

/// [`encapsulate_style`] for a specific Angular version (`None` = latest).
pub fn encapsulate_style_for_version(
    css: &str,
    component_id: &str,
    angular_version: Option<AngularVersion>,
) -> String {
    let content_attr = format!("ng-c{component_id}");
    let host_attr = format!("ng-c{component_id}");
    shim_css_text_for_version(css, &content_attr, &host_attr, angular_version)
}

// =============================================================================
// JavaScript regex semantics
// =============================================================================
//
// `ShadowCss` is written with regular expressions, and its output (whitespace
// included) follows from exactly what each one matches. The helpers below
// reproduce the JavaScript behaviour those expressions rely on.

/// JavaScript's `\s`, which is also what `String.prototype.trim` strips.
fn is_js_space(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{000B}' | '\u{000C}' | '\r' | ' ' | '\u{00A0}' | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// JavaScript's `String.prototype.trim`.
fn js_trim(s: &str) -> &str {
    s.trim_matches(is_js_space)
}

/// A JavaScript line terminator: what `.` never matches and where `^` matches
/// under the `m` flag.
fn is_js_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// Whether `s` has `pattern` at byte `at`, ignoring ASCII case (the `i` flag).
fn starts_with_ignore_ascii_case(s: &str, at: usize, pattern: &str) -> bool {
    s.as_bytes()
        .get(at..at + pattern.len())
        .is_some_and(|bytes| bytes.eq_ignore_ascii_case(pattern.as_bytes()))
}

/// The first byte offset at or after `from` where `pattern` (ASCII) occurs,
/// ignoring ASCII case.
fn find_ignore_ascii_case(s: &str, from: usize, pattern: &str) -> Option<usize> {
    let last = s.len().checked_sub(pattern.len())?;
    (from..=last).find(|&at| starts_with_ignore_ascii_case(s, at, pattern))
}

/// Replace every occurrence of `pattern` (ASCII) for which `accept` holds on the
/// text after it, ignoring ASCII case, like `s.replace(/pattern(?=...)/gi, with)`.
fn replace_ignore_ascii_case(
    s: &str,
    pattern: &str,
    with: &str,
    accept: impl Fn(&str) -> bool,
) -> String {
    let mut result = String::with_capacity(s.len());
    let mut copied = 0;
    let mut from = 0;
    while let Some(at) = find_ignore_ascii_case(s, from, pattern) {
        let end = at + pattern.len();
        if accept(&s[end..]) {
            result.push_str(&s[copied..at]);
            result.push_str(with);
            copied = end;
            from = end;
        } else {
            from = at + 1;
        }
    }
    result.push_str(&s[copied..]);
    result
}

/// For every byte offset `i` of `s` (and `s.len()`), whether the text at `i`
/// closes an enclosing parenthesis, i.e. matches
/// `[^)(]*(?:\([^)(]*(?:\([^)(]*(?:\([^)(]*\)[^)(]*)*\)[^)(]*)*\)[^)(]*)*\)`.
///
/// `_selectorSplitRe` and the combinator separator of `_applySelectorScope`
/// use it as a negative lookahead to skip commas and combinators inside
/// parentheses. Groups before the closing parenthesis may nest three deep.
///
/// Scanning forward from each offset would be quadratic, so the table is built
/// in one pass from the end: `outcome[d]` is whether the scan from the current
/// offset, already `d` groups deep, reaches the closing parenthesis.
fn closes_enclosing_paren_table(s: &str) -> Vec<bool> {
    let bytes = s.as_bytes();
    let mut table = vec![false; bytes.len() + 1];
    let mut outcome = [false; 4];
    for (i, &b) in bytes.iter().enumerate().rev() {
        outcome = match b {
            // One group deeper; a fourth level fails the match.
            b'(' => [outcome[1], outcome[2], outcome[3], false],
            // At depth 0 this is the closing parenthesis; otherwise one group up.
            b')' => [true, outcome[0], outcome[1], outcome[2]],
            _ => outcome,
        };
        table[i] = outcome[0];
    }
    table
}

/// Match Angular's `_parenSuffix`, `\(((?:\((?:\([^)(]*\)|[^)(]*)+?\)|[^)(]*)+?)\)`,
/// at byte `at`: a parenthesized group whose content nests at most two deep.
/// Returns the byte offset of the closing `)`.
fn match_paren_suffix(s: &str, at: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    if bytes.get(at) != Some(&b'(') {
        return None;
    }
    let mut depth = 0u32;
    for (i, &b) in bytes.iter().enumerate().skip(at + 1) {
        match b {
            b'(' => {
                depth += 1;
                if depth > 2 {
                    return None;
                }
            }
            b')' => {
                if depth == 0 {
                    return Some(i);
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    None
}

/// Port of `ShadowCss._splitOnTopLevelCommas`: split on commas outside
/// parentheses, stopping at an unbalanced `)` when `return_on_closing_paren`.
fn split_on_top_level_commas(text: &str, return_on_closing_paren: bool) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut parens = 0i32;
    let mut prev = 0;
    for (i, b) in text.bytes().enumerate() {
        match b {
            b'(' => parens += 1,
            b')' => {
                parens -= 1;
                if parens < 0 && return_on_closing_paren {
                    parts.push(&text[prev..i]);
                    return parts;
                }
            }
            b',' if parens == 0 => {
                parts.push(&text[prev..i]);
                prev = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&text[prev..]);
    parts
}

// =============================================================================
// Host selector processing
// =============================================================================

/// Port of `_insertPolyfillHostInCssText`: `:host-context` followed by a
/// non-empty argument list becomes `-shadowcsscontext`, then every other
/// `:host` becomes `-shadowcsshost`, ignoring case. A bare or empty
/// `:host-context` is left literal and scoped like any pseudo-class.
fn insert_polyfill_host(css: &str) -> String {
    // `/:host-context(?=\(\s*[^)\s])/gim`
    let css = replace_ignore_ascii_case(css, ":host-context", POLYFILL_HOST_CONTEXT, |after| {
        after
            .strip_prefix('(')
            .and_then(|arg| arg.trim_start_matches(is_js_space).chars().next())
            .is_some_and(|c| c != ')')
    });
    // `/:host(?!\-context)/gim`
    replace_ignore_ascii_case(&css, ":host", POLYFILL_HOST, |after| {
        !starts_with_ignore_ascii_case(after, 0, "-context")
    })
}

/// Port of `_convertColonHost`: rewrite every `-shadowcsshost(<selector>)<rest>`
/// (`_cssColonHostRe`, rest running to the next `,` or `{`) into
/// `-shadowcsshost-no-combinator<selector><rest>`.
///
/// `:host(.a)` becomes `-shadowcsshost-no-combinator.a`; a `:host(...)` whose
/// argument is a selector list is put back as a literal `:host(...)`.
fn convert_colon_host(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let mut copied = 0;
    while let Some(at) = find_ignore_ascii_case(css, copied, POLYFILL_HOST) {
        let after_marker = at + POLYFILL_HOST.len();
        let (host_selectors, other_start) = match match_paren_suffix(css, after_marker) {
            Some(close) => (&css[after_marker + 1..close], close + 1),
            None => ("", after_marker),
        };
        let other_end = css[other_start..].find([',', '{']).map_or(css.len(), |i| other_start + i);
        let other_selectors = &css[other_start..other_end];

        result.push_str(&css[copied..at]);
        let parts = split_on_top_level_commas(host_selectors, true);
        if !host_selectors.is_empty() && parts.len() > 1 {
            result.push_str(":host(");
            result.push_str(host_selectors);
            result.push(')');
        } else {
            result.push_str(POLYFILL_HOST_NO_COMBINATOR);
            let trimmed = js_trim(parts[0]);
            // `trimmedHostSelector.replace(_polyfillHost, '')`: first occurrence only.
            result.push_str(&trimmed.replacen(POLYFILL_HOST, "", 1));
        }
        result.push_str(other_selectors);
        copied = other_end;
    }
    result.push_str(&css[copied..]);
    result
}

/// Port of `_convertColonHostContext`: expand every `-shadowcsscontext(...)`
/// into the selectors `:host-context()` can match.
///
/// Like Angular, the whole stylesheet is split on top-level commas and each
/// piece is rewritten on its own; the rewrite of a piece stops at the next `{`.
fn convert_colon_host_context(css: &str) -> String {
    split_on_top_level_commas(css, false)
        .into_iter()
        .map(convert_colon_host_context_in_selector_part)
        .collect::<Vec<_>>()
        .join(",")
}

/// Port of `_convertColonHostContextInSelectorPart`: replace every match of
/// `_cssColonHostContextReGlobal`, `(:(where|is)\()?(-shadowcsscontext<parens>([^{]*))`
/// (case-insensitive), with its expansion.
fn convert_colon_host_context_in_selector_part(css: &str) -> String {
    let bytes = css.as_bytes();
    let mut result = String::with_capacity(css.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        let prefix_len = if starts_with_ignore_ascii_case(css, at, ":where(") {
            7
        } else if starts_with_ignore_ascii_case(css, at, ":is(") {
            4
        } else {
            0
        };
        let context_at = if prefix_len > 0 && is_host_context_call(css, at + prefix_len) {
            Some(at + prefix_len)
        } else if is_host_context_call(css, at) {
            Some(at)
        } else {
            None
        };
        let Some(context_at) = context_at else {
            at += 1;
            continue;
        };
        let end = css[context_at..].find('{').map_or(css.len(), |i| context_at + i);
        result.push_str(&css[copied..at]);
        result.push_str(&expand_host_context(&css[at..end], &css[at..context_at]));
        copied = end;
        at = end;
    }
    result.push_str(&css[copied..]);
    result
}

/// Whether `-shadowcsscontext(...)` (with valid `_parenSuffix` parentheses)
/// starts at byte `at`, ignoring case.
fn is_host_context_call(css: &str, at: usize) -> bool {
    starts_with_ignore_ascii_case(css, at, POLYFILL_HOST_CONTEXT)
        && match_paren_suffix(css, at + POLYFILL_HOST_CONTEXT.len()).is_some()
}

/// The replacement callback of `_convertColonHostContextInSelectorPart`.
///
/// `selector_text` is the whole match; `pseudo_prefix` is the `:where(` or
/// `:is(` it starts with, if any. Each `-shadowcsscontext(a, b)` multiplies the
/// selector groups; the text after the last one is the "other selectors".
fn expand_host_context(selector_text: &str, pseudo_prefix: &str) -> String {
    let mut groups: Vec<Vec<&str>> = vec![vec![]];
    let mut selector_text = selector_text;
    while let Some(start) = selector_text.find(POLYFILL_HOST_CONTEXT) {
        let after_prefix = &selector_text[start + POLYFILL_HOST_CONTEXT.len()..];
        let Some(args) = after_prefix.strip_prefix('(') else {
            selector_text = after_prefix;
            continue;
        };
        let mut context_selectors = Vec::new();
        let mut end = 0;
        for selector in split_on_top_level_commas(args, true) {
            end += selector.len() + 1;
            let trimmed = js_trim(selector);
            if !trimmed.is_empty() {
                context_selectors.push(trimmed);
            }
        }
        let groups_len = groups.len();
        repeat_groups(&mut groups, context_selectors.len());
        for (i, selector) in context_selectors.iter().enumerate() {
            for j in 0..groups_len {
                groups[j + i * groups_len].push(selector);
            }
        }
        // `afterPrefix.substring(endIndex + 1)`
        selector_text = after_prefix.get(end + 1..).unwrap_or("");
    }
    groups
        .into_iter()
        .map(|context_selectors| {
            combine_host_context_selectors(context_selectors, selector_text, pseudo_prefix)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Port of `repeatGroups`: append `multiples - 1` copies of the groups.
fn repeat_groups<T: Clone>(groups: &mut Vec<Vec<T>>, multiples: usize) {
    let len = groups.len();
    for _ in 1..multiples {
        for j in 0..len {
            groups.push(groups[j].clone());
        }
    }
}

/// Port of `_combineHostContextSelectors`.
///
/// For context selectors `.a`, `.b` the host may sit under or on any of them,
/// so this builds `.a.b`, `.a .b` and `.b .a` (in Angular's order), each once
/// with the host marker attached and once as a descendant, unless the other
/// selectors already contain the host.
fn combine_host_context_selectors(
    mut context_selectors: Vec<&str>,
    other_selectors: &str,
    pseudo_prefix: &str,
) -> String {
    let host_marker = POLYFILL_HOST_NO_COMBINATOR;
    let other_selectors_has_host =
        find_ignore_ascii_case(other_selectors, 0, POLYFILL_HOST).is_some();
    let Some(last) = context_selectors.pop() else {
        return format!("{host_marker}{other_selectors}");
    };
    let mut combined = vec![last.to_string()];
    while let Some(context_selector) = context_selectors.pop() {
        let len = combined.len();
        combined.resize(len * 3, String::new());
        for i in 0..len {
            let previous = std::mem::take(&mut combined[i]);
            combined[len * 2 + i] = format!("{previous} {context_selector}");
            combined[len + i] = format!("{context_selector} {previous}");
            combined[i] = format!("{context_selector}{previous}");
        }
    }
    combined
        .iter()
        .map(|s| {
            if other_selectors_has_host {
                format!("{pseudo_prefix}{s}{other_selectors}")
            } else {
                format!(
                    "{pseudo_prefix}{s}{host_marker}{other_selectors}, {pseudo_prefix}{s} {host_marker}{other_selectors}"
                )
            }
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Convert shadow DOM selectors (::shadow, ::content, etc.)
/// Note: ::ng-deep, >>>, /deep/ are handled during scoping, not here.
fn convert_shadow_dom_selectors(css: &str) -> String {
    // These shadow DOM selectors are converted to spaces
    // Note: ::ng-deep and >>> are handled in scope_selector_list, not here
    css.replace("::shadow", " ").replace("::content", " ")
}

// =============================================================================
// Selector scoping
// =============================================================================

/// At-rules whose bodies hold rules that are scoped like top-level ones.
/// Matches Angular's `scopedAtRuleIdentifiers`.
const SCOPED_AT_RULES: &[&str] =
    &["@media", "@supports", "@document", "@layer", "@container", "@scope", "@starting-style"];

/// At-rules whose bodies are kept unscoped but have `::ng-deep`, `>>>`,
/// `/deep/` and `:host` stripped (e.g. `@page { ::ng-deep @top-left {} }`).
/// Every other block at-rule (`@keyframes`, `@property`, `@counter-style`,
/// `@font-feature-values`, unknown ones...) keeps its body exactly as written.
const STRIP_SCOPING_AT_RULES: &[&str] = &["@font-face", "@page"];

/// Scope all CSS selectors with the content attribute (`_scopeSelectors`).
///
/// `css_nesting` turns on the v22.2 behaviour, see [`has_css_nesting_support`].
fn scope_selectors(css: &str, content_attr: &str, host_attr: &str, css_nesting: bool) -> String {
    let mut scoper = SelectorScoper::new(content_attr, host_attr, css_nesting);
    let mut result = String::with_capacity(css.len() * 2);
    let mut chars = css.chars().peekable();
    let mut in_at_rule_header = false;
    let mut at_rule_depth: u32 = 0;
    let mut in_string = false;
    let mut string_char = '"';
    let mut current_selector = String::new();
    let mut in_comment = false;
    let mut in_declaration = false;
    let mut declaration_depth: u32 = 0;
    let mut declaration_start = 0; // Where the current rule body starts in `result`
    let mut rule_has_deep_combinator = false; // Its selector holds `::ng-deep`/`>>>`/`/deep/`
    let mut at_rule_header = String::new(); // Collect the at-rule header text
    let mut in_unscoped_at_rule = false; // Inside an at-rule whose body is not scoped
    let mut strip_unscoped_body = false; // Strip ::ng-deep/:host from that body (@font-face/@page)
    let mut unscoped_depth: u32 = 0; // Brace depth inside the unscoped at-rule
    let mut unscoped_content = String::new(); // Collected body of the unscoped at-rule

    // Where comment and string text goes in the current state.
    macro_rules! sink {
        () => {
            if in_unscoped_at_rule {
                &mut unscoped_content
            } else if in_at_rule_header || in_declaration {
                &mut result
            } else {
                &mut current_selector
            }
        };
    }

    while let Some(c) = chars.next() {
        // Handle comments
        if !in_string && c == '/' && chars.peek() == Some(&'*') {
            in_comment = true;
        }
        if in_comment {
            let sink = sink!();
            sink.push(c);
            if c == '*' && chars.peek() == Some(&'/') {
                if let Some(next_c) = chars.next() {
                    sink.push(next_c);
                }
                in_comment = false;
            }
            continue;
        }

        // A backslash outside a string escapes the next character, as in
        // Angular's `escapeInStrings` and `escapeBlocks`: `\'`, `\{`, `\}` and
        // `\@` (Tailwind's `.font-\[\'Inter\'\]`) open no string, block or
        // at-rule. Both characters are copied as they are.
        if !in_string && c == '\\' {
            let next_c = chars.next();
            if in_at_rule_header {
                at_rule_header.push(c);
                at_rule_header.extend(next_c);
            }
            let sink = sink!();
            sink.push(c);
            sink.extend(next_c);
            // An escaped brace also ends an at-rule header: `_ruleRe` reads no
            // brace into one, so `@media x\{` stays plain text and the next
            // rule is scoped on its own.
            if in_at_rule_header && matches!(next_c, Some('{' | '}')) {
                in_at_rule_header = false;
                at_rule_header.clear();
                continue;
            }
            // A selector still ends there: `_ruleRe` reads no `{` or `}` into
            // one, so the text up to an escaped brace is left unscoped
            // (`.a\{ .b {}` only scopes ` .b`).
            if matches!(next_c, Some('{' | '}'))
                && !in_unscoped_at_rule
                && !in_at_rule_header
                && !in_declaration
            {
                result.push_str(&current_selector);
                current_selector.clear();
            }
            continue;
        }

        // Handle strings
        if in_string || c == '"' || c == '\'' {
            let sink = sink!();
            sink.push(c);
            if !in_string {
                in_string = true;
                string_char = c;
            } else if c == '\\' {
                // An escaped character (`\"`, `\'`, `\\`) never ends the string.
                if let Some(next_c) = chars.next() {
                    sink.push(next_c);
                }
            } else if c == string_char {
                in_string = false;
            }
            continue;
        }

        // Handle the body of an at-rule that is not scoped. This must be
        // checked BEFORE the @ rule handling so nested @-rules (like @top-left
        // inside @page) are collected properly
        if in_unscoped_at_rule {
            if c == '{' {
                unscoped_depth += 1;
                unscoped_content.push(c);
            } else if c == '}' {
                unscoped_depth -= 1;
                if unscoped_depth == 0 {
                    if strip_unscoped_body {
                        // Strip ::ng-deep, >>>, /deep/, and :host from the collected content
                        result.push_str(&strip_scoping_selectors(&unscoped_content));
                    } else {
                        result.push_str(&unscoped_content);
                    }
                    result.push('}');
                    unscoped_content.clear();
                    in_unscoped_at_rule = false;
                    at_rule_depth = at_rule_depth.saturating_sub(1);
                } else {
                    unscoped_content.push(c);
                }
            } else {
                unscoped_content.push(c);
            }
            continue;
        }

        // Handle @ rules (media queries, keyframes, etc.). An `@` inside a
        // declaration block or an at-rule header (`url(a@2x.png)`) does not
        // start an at-rule; it is copied by the branches below.
        if c == '@' && !in_declaration && !in_at_rule_header {
            // Flush any pending selector
            if !current_selector.is_empty() {
                result.push_str(&current_selector);
                current_selector.clear();
            }
            in_at_rule_header = true;
            at_rule_header.clear();
            at_rule_header.push(c);
            result.push(c);
            continue;
        }

        if in_at_rule_header {
            at_rule_header.push(c);
            result.push(c);
            if c == '{' {
                at_rule_depth += 1;
                in_at_rule_header = false;

                // Like Angular, only the listed at-rules get their body
                // scoped, matched case-sensitively on the header's start.
                if !SCOPED_AT_RULES.iter().any(|r| at_rule_header.starts_with(r)) {
                    in_unscoped_at_rule = true;
                    strip_unscoped_body =
                        STRIP_SCOPING_AT_RULES.iter().any(|r| at_rule_header.starts_with(r));
                    unscoped_depth = 1;
                }
                at_rule_header.clear();
            } else if c == ';' {
                // @ rules that end with ; like @import
                in_at_rule_header = false;
                at_rule_header.clear();
            }
            continue;
        }

        // Track at-rule nesting for closing braces
        if at_rule_depth > 0 && c == '}' && !in_declaration {
            // Text with no `{` before the `}` (`@media x { .a {} b }`) is not
            // a rule; Angular leaves it unscoped, in place.
            result.push_str(&current_selector);
            current_selector.clear();
            at_rule_depth = at_rule_depth.saturating_sub(1);
            result.push(c);
            continue;
        }

        // If inside a declaration block, just copy content directly
        if in_declaration {
            if c == '{' {
                declaration_depth += 1;
            } else if c == '}' {
                declaration_depth -= 1;
                if declaration_depth == 0 {
                    in_declaration = false;
                    // From v22.2 a body is either scoped as nested rules or has
                    // its scoping selectors stripped. Nested rules are not
                    // scoped yet, so such a body is kept as written.
                    if css_nesting
                        && (rule_has_deep_combinator
                            || !has_nested_block(&result[declaration_start..]))
                    {
                        let body = result.split_off(declaration_start);
                        result.push_str(&strip_scoping_selectors(&body));
                    }
                }
            }
            result.push(c);
            continue;
        }

        // Regular selector parsing
        if c == '{' {
            // End of selector, scope it
            rule_has_deep_combinator = contains_deep_combinator(&current_selector);
            let scoped = scope_rule_selector(&current_selector, &mut scoper);
            result.push_str(&scoped);
            result.push('{');
            current_selector.clear();
            in_declaration = true;
            declaration_depth = 1;
            declaration_start = result.len();
        } else if c == '}' {
            // A stray `}`: keep the text before it unscoped, in place.
            result.push_str(&current_selector);
            current_selector.clear();
            result.push(c);
        } else {
            current_selector.push(c);
        }
    }

    // Handle any remaining content
    if !current_selector.trim().is_empty() {
        result.push_str(&current_selector);
    }

    result
}

// =============================================================================
// Selector scoping - port of `_scopeSelector` and `_applySelectorScope`
// =============================================================================

/// Whether this Angular version's `ShadowCss` has its CSS nesting support
/// (v22.2.0-next.2+).
///
/// From that release a selector part that is just `&` is left unscoped, a rule
/// without a declaration block (`.a;`) is no longer scoped, and a rule body
/// with no nested rule (or one under a `::ng-deep` selector) has its scoping
/// selectors stripped like an `@font-face` body. Oxc does not scope nested
/// rules yet, so a body holding them is kept as written.
fn has_css_nesting_support(version: Option<AngularVersion>) -> bool {
    let Some(version) = version else {
        return true;
    };
    if (version.major, version.minor, version.patch) == (22, 2, 0) {
        return match version.prerelease {
            None | Some(Prerelease::Rc(_)) => true,
            Some(Prerelease::Next(n)) => n >= 2,
            Some(Prerelease::Other) => false,
        };
    }
    version.is_at_least(22, 2, 0)
}

/// Scopes the selectors of one stylesheet, carrying the state Angular keeps on
/// its `ShadowCss` instance (`_shouldScopeIndicator`, `_safeSelector`).
struct SelectorScoper<'a> {
    /// The content attribute, e.g. `_ngcontent-%COMP%` (`scopeSelector`).
    scope_selector: &'a str,
    /// The host attribute, e.g. `_nghost-%COMP%` (`hostSelector`).
    host_selector: &'a str,
    /// `[scopeSelector]` with any `[is=...]` unwrapped, appended to scoped parts.
    attr_name: String,
    /// Whether a selector part that is only `&` stays unscoped (v22.2+).
    css_nesting: bool,
    /// `_shouldScopeIndicator`: whether parts are scoped. Parts before a
    /// `:host` match ancestors of the host and stay unscoped.
    should_scope: bool,
    /// `_safeSelector` of the current top-level selector.
    safe_selector: SafeSelector,
}

impl<'a> SelectorScoper<'a> {
    fn new(scope_selector: &'a str, host_selector: &'a str, css_nesting: bool) -> Self {
        Self {
            scope_selector,
            host_selector,
            attr_name: format!("[{}]", unwrap_is_attribute(scope_selector)),
            css_nesting,
            should_scope: false,
            safe_selector: SafeSelector::default(),
        }
    }

    /// Port of `_scopeSelector`: scope each selector of a list (`, `-joined),
    /// leaving everything after a `::ng-deep`, `>>>` or `/deep/` unscoped.
    fn scope_selector(&mut self, selector: &str, is_parent_selector: bool) -> String {
        let mut result = String::with_capacity(selector.len() * 2);
        for (i, part) in split_selector_list(selector).into_iter().enumerate() {
            if i > 0 {
                result.push_str(", ");
            }
            let mut deep_parts = split_on_deep_combinators(part).into_iter();
            let shallow_part = deep_parts.next().unwrap_or_default();
            if self.selector_needs_scoping(shallow_part) {
                result.push_str(&self.apply_selector_scope(shallow_part, is_parent_selector));
            } else {
                result.push_str(shallow_part);
            }
            for other_part in deep_parts {
                result.push(' ');
                result.push_str(other_part);
            }
        }
        result
    }

    /// Port of `_selectorNeedsScoping`: false when some line of the selector
    /// already starts with the content attribute (`_makeScopeMatcher` builds
    /// `^(attr)([>\s~+[.,{:][\s\S]*)?$` with the `m` flag).
    fn selector_needs_scoping(&self, selector: &str) -> bool {
        let mut line = selector;
        loop {
            if let Some(after) = line.strip_prefix(self.scope_selector) {
                match after.chars().next() {
                    None => return false,
                    Some(c)
                        if is_js_space(c)
                            || matches!(c, '>' | '~' | '+' | '[' | '.' | ',' | '{' | ':') =>
                    {
                        return false;
                    }
                    Some(_) => {}
                }
            }
            match line.char_indices().find(|&(_, c)| is_js_line_terminator(c)) {
                Some((i, c)) => line = &line[i + c.len_utf8()..],
                None => return true,
            }
        }
    }

    /// Port of `_applySelectorScope`.
    ///
    /// The selector is split on the combinator separator
    /// `/( |>|\+|~(?!=))(?!<inside parens>)\s*/g`, and every part is emitted as
    /// `` `${scopedPart} ${separator} ` ``. That is where Angular's runs of
    /// spaces come from: `a > b` is split at the space (`a`) and again at the
    /// `>` (an empty part), giving `a[c]    > b[c]`.
    fn apply_selector_scope(&mut self, selector: &str, is_parent_selector: bool) -> String {
        let escaped;
        let selector = if is_parent_selector {
            self.safe_selector = SafeSelector::new(selector);
            escaped = self.safe_selector.content.clone();
            escaped.as_str()
        } else {
            selector
        };

        let has_host = selector.contains(POLYFILL_HOST_NO_COMBINATOR);
        // Only reset at the top level, or while scoping is on.
        if is_parent_selector || self.should_scope {
            self.should_scope = !has_host;
        }

        let mut scoped = String::with_capacity(selector.len() * 2);
        // Oxc deviation (#175): a newline, tab, carriage return or form feed
        // is a descendant combinator too. Angular only splits on a space, so
        // `:host:hover\n.child` leaves `.child` unscoped and `.foo\n.bar` keeps
        // `.foo` unscoped. Whitespace that starts the selector (`.a,\n.b`) is
        // not a combinator, so it is kept exactly as Angular keeps it.
        // The extra split is skipped inside a part Angular would scope as one
        // when that part has host markers that only work together (see
        // `can_split_on_other_whitespace`); such a part is left as Angular has it.
        let leading = selector.len() - selector.trim_start_matches(is_js_space).len();
        let mut start = 0;
        let mut from = 0;
        // Start of the part Angular's own split is in, and whether the Oxc-only
        // split is allowed in it (worked out on the first newline or tab).
        let mut angular_part_start = 0;
        let mut other_whitespace_split: Option<bool> = None;
        let closes_paren = closes_enclosing_paren_table(selector);
        while let Some((at, separator, end)) =
            find_combinator_separator(selector, &closes_paren, from, leading)
        {
            from = end;
            let is_other_whitespace =
                matches!(selector.as_bytes()[at], b'\n' | b'\t' | b'\r' | b'\x0C');
            if is_other_whitespace
                && !*other_whitespace_split.get_or_insert_with(|| {
                    let angular_part_end =
                        find_angular_part_end(selector, &closes_paren, angular_part_start, at);
                    can_split_on_other_whitespace(&selector[angular_part_start..angular_part_end])
                })
            {
                continue;
            }
            // A space between a hex escape and a following hex digit (`.\fc ber`
            // for `.über`) terminates the escape; it is not a combinator.
            if is_escaped_hex_space(selector, start, at) {
                continue;
            }
            let part = &selector[start..at];
            let scoped_part = self.scope_pseudo_function_aware_part(part);
            scoped.push_str(&scoped_part);
            scoped.push(' ');
            scoped.push(separator);
            scoped.push(' ');
            start = end;
            if !is_other_whitespace {
                angular_part_start = end;
                other_whitespace_split = None;
            }
        }
        let scoped_part = self.scope_pseudo_function_aware_part(&selector[start..]);
        scoped.push_str(&scoped_part);
        self.safe_selector.restore(&scoped)
    }

    /// Port of `_pseudoFunctionAwareScopeSelectorPart`: a part made only of
    /// `:where(...)`/`:is(...)` has the selectors inside scoped instead.
    fn scope_pseudo_function_aware_part(&mut self, part: &str) -> String {
        // Oxc deviation: comment placeholders are detached first. Angular scopes
        // them as selector text (`.a /* c */ .b` gets a bare `[content]` part
        // for the comment, and a comment glued to `:where(.one)` turns it into
        // `[content]:where(.one)`, which changes specificity). The placeholders
        // restore to blank text or the comment, so they go back in front.
        if part.contains(COMMENT_PLACEHOLDER_PREFIX) {
            let (stripped, placeholders) = strip_comment_placeholders(part);
            if !placeholders.is_empty() {
                if js_trim(&stripped).is_empty() {
                    return part.to_string();
                }
                return placeholders + &self.scope_pseudo_function_aware_part(&stripped);
            }
        }

        // `_cssPrefixWithPseudoSelectorFunction.exec` loop, collecting each
        // `:where(`/`:is(` up to its balancing `)`.
        let bytes = part.as_bytes();
        let mut pseudo_parts: Vec<(usize, usize)> = Vec::new();
        let mut last_index = 0;
        while let Some((start, open_end)) = find_where_or_is(part, last_index) {
            let mut opened = 1;
            let mut index = open_end;
            while index < bytes.len() {
                let c = bytes[index];
                index += 1;
                if c == b'(' {
                    opened += 1;
                } else if c == b')' {
                    opened -= 1;
                    if opened == 0 {
                        break;
                    }
                }
            }
            pseudo_parts.push((start, index));
            last_index = index;
        }

        // `pseudoSelectorParts.join('') === selectorPart`
        let covers_part = pseudo_parts.first().map_or(0, |&(start, _)| start) == 0
            && pseudo_parts.windows(2).all(|w| w[0].1 == w[1].0)
            && pseudo_parts.last().map_or(0, |&(_, end)| end) == part.len();
        if covers_part {
            let mut scoped = String::with_capacity(part.len() * 2);
            for &(start, end) in &pseudo_parts {
                let pseudo_part = &part[start..end];
                let function_len =
                    if starts_with_ignore_ascii_case(pseudo_part, 0, ":where(") { 7 } else { 4 };
                // `selectorPart.slice(cssPseudoSelectorFunction.length, -1)`
                let inner_end =
                    pseudo_part.char_indices().next_back().map_or(0, |(i, _)| i).max(function_len);
                let selector_to_scope = &pseudo_part[function_len..inner_end];
                if selector_to_scope.contains(POLYFILL_HOST_NO_COMBINATOR) {
                    self.should_scope = true;
                }
                let scoped_inner = self.scope_selector(selector_to_scope, false);
                scoped.push_str(&pseudo_part[..function_len]);
                scoped.push_str(&scoped_inner);
                scoped.push(')');
            }
            scoped
        } else {
            self.should_scope = self.should_scope || part.contains(POLYFILL_HOST_NO_COMBINATOR);
            if self.should_scope { self.scope_selector_part(part) } else { part.to_string() }
        }
    }

    /// Port of the `_scopeSelectorPart` closure of `_applySelectorScope`.
    fn scope_selector_part(&self, part: &str) -> String {
        let trimmed = js_trim(part);
        if trimmed.is_empty() || (self.css_nesting && trimmed == "&") {
            return part.to_string();
        }
        if part.contains(POLYFILL_HOST_NO_COMBINATOR) {
            let scoped = apply_simple_selector_scope(part, self.host_selector);
            if has_host_marker_outside_parens(part) {
                scoped
            } else {
                // A host marker inside a pseudo-function, e.g. `:not(:host)`:
                // the part also gets the content attribute.
                insert_before_first_colon(&scoped, &self.attr_name)
            }
        } else {
            let without_host = replace_ignore_ascii_case(part, POLYFILL_HOST, "", |_| true);
            if without_host.is_empty() {
                trimmed.to_string()
            } else {
                insert_before_first_colon(&without_host, &self.attr_name)
            }
        }
    }
}

/// `scopeSelector.replace(/\[is=([^\]]*)\]/g, (_, ...parts) => parts[0])`.
fn unwrap_is_attribute(scope_selector: &str) -> String {
    let mut result = String::with_capacity(scope_selector.len());
    let mut rest = scope_selector;
    while let Some(at) = rest.find("[is=") {
        let value_start = at + "[is=".len();
        let Some(close) = rest[value_start..].find(']') else {
            break;
        };
        result.push_str(&rest[..at]);
        result.push_str(&rest[value_start..value_start + close]);
        rest = &rest[value_start + close + 1..];
    }
    result.push_str(rest);
    result
}

/// Split a selector list on `_selectorSplitRe`: a comma outside parentheses
/// together with one optional space on either side. Other whitespace around
/// the comma (a newline, say) stays in the selectors.
fn split_selector_list(selector: &str) -> Vec<&str> {
    let bytes = selector.as_bytes();
    let closes_paren = closes_enclosing_paren_table(selector);
    let mut parts = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let comma = if bytes[at] == b' ' && bytes.get(at + 1) == Some(&b',') {
            at + 1
        } else if bytes[at] == b',' {
            at
        } else {
            at += 1;
            continue;
        };
        if closes_paren[comma + 1] {
            at += 1;
            continue;
        }
        let end = if bytes.get(comma + 1) == Some(&b' ') { comma + 2 } else { comma + 1 };
        parts.push(&selector[start..at]);
        start = end;
        at = end;
    }
    parts.push(&selector[start..]);
    parts
}

/// `part.split(/(?:>>>)|(?:\/deep\/)|(?:::ng-deep)/g)`.
fn split_on_deep_combinators(part: &str) -> Vec<&str> {
    let bytes = part.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let len = deep_combinator_len(&bytes[at..]);
        if len > 0 {
            parts.push(&part[start..at]);
            at += len;
            start = at;
        } else {
            at += 1;
        }
    }
    parts.push(&part[start..]);
    parts
}

/// The length of the `>>>`, `/deep/` or `::ng-deep` that `bytes` starts with, or 0.
fn deep_combinator_len(bytes: &[u8]) -> usize {
    if bytes.starts_with(b">>>") {
        3
    } else if bytes.starts_with(b"/deep/") {
        6
    } else if bytes.starts_with(b"::ng-deep") {
        9
    } else {
        0
    }
}

/// Whether a selector contains `::ng-deep`, `>>>` or `/deep/`.
fn contains_deep_combinator(selector: &str) -> bool {
    let bytes = selector.as_bytes();
    (0..bytes.len()).any(|at| deep_combinator_len(&bytes[at..]) > 0)
}

/// Find the next combinator separator at or after byte `from`, matching
/// `/( |>|\+|~(?!=))(?!<closes an enclosing paren>)\s*/g`, plus (from byte
/// `other_whitespace_from` on) a newline, tab, carriage return or form feed,
/// which is reported as a space.
/// `closes_paren` is [`closes_enclosing_paren_table`] of `selector`.
/// Returns its start, the separator character and the end of the trailing
/// whitespace.
fn find_combinator_separator(
    selector: &str,
    closes_paren: &[bool],
    from: usize,
    other_whitespace_from: usize,
) -> Option<(usize, char, usize)> {
    let bytes = selector.as_bytes();
    (from..bytes.len()).find_map(|at| {
        let b = bytes[at];
        let separator = match b {
            b' ' | b'>' | b'+' => b,
            b'~' if bytes.get(at + 1) != Some(&b'=') => b,
            b'\n' | b'\t' | b'\r' | b'\x0C' if at >= other_whitespace_from => b' ',
            _ => return None,
        };
        if closes_paren[at + 1] {
            return None;
        }
        let rest = &selector[at + 1..];
        let end = selector.len() - rest.trim_start_matches(is_js_space).len();
        Some((at, char::from(separator), end))
    })
}

/// Whether the separator at byte `at` is a space ending a hex escape in the part
/// that starts at byte `start` (`.\fc ber` for `.über`), so not a combinator.
fn is_escaped_hex_space(selector: &str, start: usize, at: usize) -> bool {
    has_escape_placeholder(&selector[start..at])
        && selector.as_bytes().get(at + 1).is_some_and(u8::is_ascii_hexdigit)
}

/// End of the part Angular's own split (`/( |>|\+|~(?!=))…\s*/`, no newline or
/// tab) would produce for the part starting at byte `start`, given that byte
/// `at` is inside it. `closes_paren` is [`closes_enclosing_paren_table`] of `selector`.
fn find_angular_part_end(selector: &str, closes_paren: &[bool], start: usize, at: usize) -> usize {
    let mut from = at;
    while let Some((end_at, _, end)) =
        find_combinator_separator(selector, closes_paren, from, usize::MAX)
    {
        if !is_escaped_hex_space(selector, start, end_at) {
            return end_at;
        }
        from = end;
    }
    selector.len()
}

/// Whether the Oxc-only newline/tab split may cut `part`, a part Angular scopes
/// as one.
///
/// Angular only turns a bare `-shadowcsshost` (the rest of a selector after a
/// first `:host(...)`, as in `:host(.a)\n:not(:host)`) into the host attribute
/// while it sits in a part with a `-shadowcsshost-no-combinator` marker. And a
/// part whose only host marker is inside a pseudo-function (`:where(:host)`)
/// gets the content attribute in front. Cutting such a part would leak the
/// marker or scope the host as content, so it is kept whole.
fn can_split_on_other_whitespace(part: &str) -> bool {
    let mut from = 0;
    while let Some(at) = find_ignore_ascii_case(part, from, POLYFILL_HOST) {
        from = at + POLYFILL_HOST.len();
        if !starts_with_ignore_ascii_case(part, at, POLYFILL_HOST_NO_COMBINATOR) {
            return false;
        }
        // `_polyfillHostNoCombinatorOutsidePseudoFunction`: `(?![^(]*\))`.
        let after = &part[at + POLYFILL_HOST_NO_COMBINATOR.len()..];
        if after.bytes().find(|&b| b == b'(' || b == b')') == Some(b')') {
            return false;
        }
    }
    true
}

/// Whether `s` contains a `SafeSelector` escape placeholder (`/__esc-ph-(\d+)__/`).
fn has_escape_placeholder(s: &str) -> bool {
    s.match_indices("__esc-ph-").any(|(at, m)| {
        let rest = &s[at + m.len()..];
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        digits > 0 && rest[digits..].starts_with("__")
    })
}

/// Find `:where(` or `:is(` (ignoring case) at or after byte `from`.
/// Returns the match start and the offset just past its `(`.
fn find_where_or_is(s: &str, from: usize) -> Option<(usize, usize)> {
    let bytes = s.as_bytes();
    (from..bytes.len()).find_map(|at| {
        if bytes[at] != b':' {
            None
        } else if starts_with_ignore_ascii_case(s, at, ":where(") {
            Some((at, at + 7))
        } else if starts_with_ignore_ascii_case(s, at, ":is(") {
            Some((at, at + 4))
        } else {
            None
        }
    })
}

/// `s.match(/([^:]*)(:*)([\s\S]*)/)` reassembled as `before + attr + colons + after`.
fn insert_before_first_colon(s: &str, attr: &str) -> String {
    let at = s.find(':').unwrap_or(s.len());
    format!("{}{attr}{}", &s[..at], &s[at..])
}

/// Whether some host marker in `part` is outside parentheses: the
/// `-shadowcsshost-no-combinator(?![^(]*\))` test of `_scopeSelectorPart`
/// (a marker is inside when a `)` comes before the next `(`).
fn has_host_marker_outside_parens(part: &str) -> bool {
    part.match_indices(POLYFILL_HOST_NO_COMBINATOR).any(|(at, m)| {
        let rest = &part[at + m.len()..];
        match (rest.find('('), rest.find(')')) {
            (_, None) => true,
            (None, Some(_)) => false,
            (Some(open), Some(close)) => open < close,
        }
    })
}

/// Port of `_applySimpleSelectorScope` for a part holding a host marker:
/// each `-shadowcsshost-no-combinator<compound>` becomes the compound with
/// `[host]` inserted before its first `:` or `)`, then any remaining
/// `-shadowcsshost` becomes `[host]`.
fn apply_simple_selector_scope(selector: &str, host_selector: &str) -> String {
    let replace_by = format!("[{host_selector}]");
    let mut result = selector.to_string();
    // `_polyfillHostNoCombinatorRe`: `/-shadowcsshost-no-combinator([^\s,]*)/`
    while let Some(at) = result.find(POLYFILL_HOST_NO_COMBINATOR) {
        let compound_start = at + POLYFILL_HOST_NO_COMBINATOR.len();
        let compound_end = result[compound_start..]
            .find(|c: char| is_js_space(c) || c == ',')
            .map_or(result.len(), |i| compound_start + i);
        let compound = &result[compound_start..compound_end];
        // `/([^:\)]*)(:*)(.*)/` -> `before + replaceBy + colon + after`
        let split = compound.find([':', ')']).unwrap_or(compound.len());
        let replaced = format!("{}{replace_by}{}", &compound[..split], &compound[split..]);
        result.replace_range(at..compound_end, &replaced);
    }
    replace_ignore_ascii_case(&result, POLYFILL_HOST, &replace_by, |_| true)
}

/// Scope the selector of one rule like `_scopeSelectors` does.
///
/// `raw` is the text between the previous rule and this rule's `{`. Like
/// `processRules`' `_ruleRe`, it is split into rules at `;`, and each rule
/// keeps its leading whitespace and comments and its trailing whitespace
/// around the selector. Only the last one owns the block; the rules before it
/// have none and are only scoped before v22.2.
fn scope_rule_selector(raw: &str, scoper: &mut SelectorScoper) -> String {
    let segments = split_outside_strings(raw, b';');
    let mut result = String::with_capacity(raw.len() * 2);
    let last = segments.len() - 1;
    for (i, segment) in segments.into_iter().enumerate() {
        if i > 0 {
            result.push(';');
        }
        if i < last && scoper.css_nesting {
            result.push_str(segment);
        } else {
            result.push_str(&scope_one_rule_selector(segment, scoper));
        }
    }
    result
}

/// Scope the selector inside `raw` (`_ruleRe`'s
/// `(\s*(?:%COMMENT%\s*)*)([^;\{\}]+?)(\s*)`), keeping the text around it.
fn scope_one_rule_selector(raw: &str, scoper: &mut SelectorScoper) -> String {
    let mut lead = 0;
    loop {
        lead = raw.len() - raw[lead..].trim_start_matches(is_js_space).len();
        match find_comment_placeholder(raw, lead) {
            Some((range, _)) if range.start == lead => lead = range.end,
            _ => break,
        }
    }
    let end = raw.trim_end_matches(is_js_space).len();
    if lead < end {
        let selector = &raw[lead..end];
        let scoped = if selector.contains(['"', '\'']) {
            unescape_in_strings(&scoper.scope_selector(&escape_in_strings(selector), true))
        } else {
            scoper.scope_selector(selector, true)
        };
        return format!("{}{scoped}{}", &raw[..lead], &raw[end..]);
    }
    // Only whitespace and comments: the regex backtracks so that the selector
    // is the last whitespace character. A trailing comment placeholder is left
    // alone (see `scope_pseudo_function_aware_part`).
    match raw.char_indices().next_back() {
        Some((at, c)) if is_js_space(c) => {
            let scoped = scoper.scope_selector(&raw[at..], true);
            format!("{}{scoped}", &raw[..at])
        }
        _ => raw.to_string(),
    }
}

/// For each byte of `css`, whether it is inside a quoted string the way
/// `escapeInStrings` sees it (a backslash skips the next character; the
/// quotes themselves are outside).
fn string_mask(css: &str) -> Vec<bool> {
    let bytes = css.as_bytes();
    let mut mask = vec![false; bytes.len()];
    let mut quote = None;
    let mut at = 0;
    while at < bytes.len() {
        let b = bytes[at];
        if b == b'\\' {
            at += 2;
            continue;
        }
        match quote {
            Some(q) if b == q => quote = None,
            Some(_) => mask[at] = true,
            None if b == b'"' || b == b'\'' => quote = Some(b),
            None => {}
        }
        at += 1;
    }
    mask
}

const SEMI_IN_PLACEHOLDER: &str = "%SEMI_IN_PLACEHOLDER%";
const COMMA_IN_PLACEHOLDER: &str = "%COMMA_IN_PLACEHOLDER%";
const COLON_IN_PLACEHOLDER: &str = "%COLON_IN_PLACEHOLDER%";

/// Port of `escapeInStrings` for a selector: `processRules` hides the `;`, `,`
/// and `:` inside quoted strings, so `a[title="a,b"]` is not split at the comma.
fn escape_in_strings(selector: &str) -> String {
    let mask = string_mask(selector);
    let mut escaped = String::with_capacity(selector.len());
    for (at, c) in selector.char_indices() {
        match c {
            ';' if mask[at] => escaped.push_str(SEMI_IN_PLACEHOLDER),
            ',' if mask[at] => escaped.push_str(COMMA_IN_PLACEHOLDER),
            ':' if mask[at] => escaped.push_str(COLON_IN_PLACEHOLDER),
            _ => escaped.push(c),
        }
    }
    escaped
}

/// Port of `unescapeInStrings`, undoing [`escape_in_strings`].
fn unescape_in_strings(selector: &str) -> String {
    selector
        .replace(COMMA_IN_PLACEHOLDER, ",")
        .replace(SEMI_IN_PLACEHOLDER, ";")
        .replace(COLON_IN_PLACEHOLDER, ":")
}

/// Split `css` at every `delimiter` outside quoted strings.
fn split_outside_strings(css: &str, delimiter: u8) -> Vec<&str> {
    let mask = string_mask(css);
    let mut parts = Vec::new();
    let mut start = 0;
    for (at, &b) in css.as_bytes().iter().enumerate() {
        if b == delimiter && !mask[at] {
            parts.push(&css[start..at]);
            start = at + 1;
        }
    }
    parts.push(&css[start..]);
    parts
}

/// Whether a rule body holds a nested block (`rule.content.includes('{')`,
/// braces inside strings excluded).
fn has_nested_block(body: &str) -> bool {
    let mask = string_mask(body);
    body.bytes().enumerate().any(|(at, b)| b == b'{' && !mask[at])
}

/// Port of `_stripScopingSelectors`, used for bodies that cannot be scoped
/// (`@font-face`, `@page` and, from v22.2, rule bodies).
///
/// In every rule of `css` (nested blocks are left alone) `>>>`, `/deep/` and
/// `::ng-deep` become a space, and then so does the first host marker
/// together with the compound selector attached to it.
fn strip_scoping_selectors(css: &str) -> String {
    let mask = string_mask(css);
    let bytes = css.as_bytes();
    let mut result = String::with_capacity(css.len());
    let mut depth = 0u32;
    let mut segment_start = 0;
    let mut at = 0;
    while at < bytes.len() {
        let b = bytes[at];
        if b == b'\\' {
            at += 2;
            continue;
        }
        if !mask[at] {
            match b {
                b'{' => {
                    if depth == 0 {
                        result.push_str(&strip_rule_scoping(
                            &css[segment_start..at],
                            &mask[segment_start..at],
                        ));
                        segment_start = at;
                    }
                    depth += 1;
                }
                b'}' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        result.push_str(&css[segment_start..=at]);
                        segment_start = at + 1;
                    }
                }
                b'}' | b';' if depth == 0 => {
                    result.push_str(&strip_rule_scoping(
                        &css[segment_start..at],
                        &mask[segment_start..at],
                    ));
                    segment_start = at;
                }
                _ => {}
            }
        }
        at += 1;
    }
    if depth == 0 {
        result.push_str(&strip_rule_scoping(&css[segment_start..], &mask[segment_start..]));
    } else {
        result.push_str(&css[segment_start..]);
    }
    result
}

/// `rule.selector.replace(_shadowDeepSelectors, ' ').replace(_polyfillHostNoCombinatorRe, ' ')`
/// on one rule's text. `mask` marks the bytes inside strings, where Angular has
/// replaced `:` and `,` with placeholders, so `::ng-deep` is not matched there
/// and `,` does not end the host marker's compound selector.
fn strip_rule_scoping(rule: &str, mask: &[bool]) -> String {
    let bytes = rule.as_bytes();
    let mut stripped = String::with_capacity(rule.len());
    let mut stripped_mask = Vec::with_capacity(rule.len());
    let mut copied = 0;
    let mut at = 0;
    while at < bytes.len() {
        let len = match deep_combinator_len(&bytes[at..]) {
            9 if mask[at] || mask[at + 1] => 0,
            len => len,
        };
        if len == 0 {
            at += 1;
            continue;
        }
        stripped.push_str(&rule[copied..at]);
        stripped_mask.extend_from_slice(&mask[copied..at]);
        stripped.push(' ');
        stripped_mask.push(false);
        at += len;
        copied = at;
    }
    stripped.push_str(&rule[copied..]);
    stripped_mask.extend_from_slice(&mask[copied..]);

    if let Some(at) = stripped.find(POLYFILL_HOST_NO_COMBINATOR) {
        let compound_start = at + POLYFILL_HOST_NO_COMBINATOR.len();
        let compound_end = stripped[compound_start..]
            .char_indices()
            .find(|&(i, c)| is_js_space(c) || (c == ',' && !stripped_mask[compound_start + i]))
            .map_or(stripped.len(), |(i, _)| compound_start + i);
        stripped.replace_range(at..compound_end, " ");
    }
    stripped
}

// ============================================================================
// Polyfill Directive Processing
// ============================================================================

/// Process styles to convert native ShadowDOM rules that will trip up the CSS parser.
///
/// Converts polyfill-next-selector rules like:
/// ```css
/// polyfill-next-selector { content: ':host menu-item'; }
/// ::content menu-item { ... }
/// ```
///
/// to:
/// ```css
/// :host menu-item { ... }
/// ```
fn insert_polyfill_directives(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Case-insensitive check for "polyfill-next-selector"
        // Only check if we're at a valid char boundary to avoid panics with UTF-8
        if i + 22 <= len
            && css.is_char_boundary(i)
            && css.is_char_boundary(i + 22)
            && css[i..i + 22].eq_ignore_ascii_case("polyfill-next-selector")
        {
            // Find the content value and the next opening brace
            if let Some((content, skip_to)) = parse_polyfill_next_selector(&css[i..]) {
                result.push_str(&content);
                result.push_str(" {");
                i += skip_to;
                continue;
            }
        }
        i += push_utf8_char(&mut result, css, i);
    }

    result
}

/// Parse a polyfill-next-selector block and return (content_value, chars_to_skip).
/// The pattern is: polyfill-next-selector { content: 'value'; } ... {
fn parse_polyfill_next_selector(s: &str) -> Option<(String, usize)> {
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Skip "polyfill-next-selector"
    let mut i = 22;

    // Find the opening brace of the polyfill block
    while i < len && bytes[i] != b'{' {
        i += 1;
    }
    if i >= len {
        return None;
    }
    i += 1; // skip {

    // Find "content:" (case-insensitive)
    while i + 8 <= len {
        if s[i..].to_ascii_lowercase().starts_with("content") {
            let after_content = i + 7;
            // Skip whitespace and colon
            let mut j = after_content;
            while j < len && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < len && bytes[j] == b':' {
                j += 1;
                // Skip whitespace after colon
                while j < len && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }

                // Extract the quoted content
                if j < len && (bytes[j] == b'\'' || bytes[j] == b'"') {
                    let quote = bytes[j];
                    let content_start = j + 1;
                    let mut content_end = content_start;
                    while content_end < len && bytes[content_end] != quote {
                        content_end += 1;
                    }
                    let content = s[content_start..content_end].to_string();

                    // Find the closing brace of polyfill block
                    let mut k = content_end + 1;
                    while k < len && bytes[k] != b'}' {
                        k += 1;
                    }
                    if k >= len {
                        return None;
                    }
                    k += 1; // skip }

                    // Find the next opening brace (the actual rule's opening brace)
                    while k < len && bytes[k] != b'{' {
                        k += 1;
                    }
                    if k >= len {
                        return None;
                    }
                    k += 1; // skip {

                    return Some((content, k));
                }
            }
        }
        i += 1;
    }

    None
}

/// Process styles to add rules which will only apply under the polyfill.
///
/// Converts polyfill-rule rules like:
/// ```css
/// polyfill-rule {
///   content: ':host menu-item';
///   background: blue;
/// }
/// ```
///
/// to:
/// ```css
/// :host menu-item { background: blue; }
/// ```
fn insert_polyfill_rules(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Case-insensitive check for "polyfill-rule" (but not "polyfill-rule-unscoped")
        // Only check if we're at a valid char boundary to avoid panics with UTF-8
        if i + 13 <= len
            && css.is_char_boundary(i)
            && css.is_char_boundary(i + 13)
            && css[i..i + 13].eq_ignore_ascii_case("polyfill-rule")
            && (i + 13 >= len || !css[i + 13..].to_ascii_lowercase().starts_with("-unscoped"))
        {
            // Make sure it's not "polyfill-next-selector"
            if !css[i..].to_ascii_lowercase().starts_with("polyfill-next-selector") {
                if let Some((selector, body, skip_to)) = parse_polyfill_rule(&css[i..]) {
                    let cleaned_body = body.trim().trim_start_matches(';').trim();
                    result.push_str(&selector);
                    result.push_str(" { ");
                    result.push_str(cleaned_body);
                    result.push_str(" }");
                    i += skip_to;
                    continue;
                }
            }
        }
        i += push_utf8_char(&mut result, css, i);
    }

    result
}

/// Parse a polyfill-rule block and return (selector, body, chars_to_skip).
/// The pattern is: polyfill-rule { content: 'selector'; ...body... }
fn parse_polyfill_rule(s: &str) -> Option<(String, String, usize)> {
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Skip "polyfill-rule"
    let mut i = 13;

    // Find the opening brace
    while i < len && bytes[i] != b'{' {
        i += 1;
    }
    if i >= len {
        return None;
    }
    i += 1; // skip {

    // Find "content:" (case-insensitive)
    let mut content_found = false;
    let mut selector = String::new();
    let mut body_start = i;

    while i + 8 <= len {
        if s[i..].to_ascii_lowercase().starts_with("content") {
            let after_content = i + 7;
            let mut j = after_content;
            while j < len && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < len && bytes[j] == b':' {
                j += 1;
                while j < len && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }

                if j < len && (bytes[j] == b'\'' || bytes[j] == b'"') {
                    let quote = bytes[j];
                    let content_start = j + 1;
                    let mut content_end = content_start;
                    while content_end < len && bytes[content_end] != quote {
                        content_end += 1;
                    }
                    selector = s[content_start..content_end].to_string();
                    content_found = true;

                    // Skip past the closing quote and any semicolon/whitespace
                    body_start = content_end + 1;
                    while body_start < len
                        && (bytes[body_start].is_ascii_whitespace() || bytes[body_start] == b';')
                    {
                        body_start += 1;
                    }
                    break;
                }
            }
        }
        i += 1;
    }

    if !content_found {
        return None;
    }

    // Find the closing brace
    let mut body_end = body_start;
    let mut brace_depth = 1;
    while body_end < len && brace_depth > 0 {
        if bytes[body_end] == b'{' {
            brace_depth += 1;
        } else if bytes[body_end] == b'}' {
            brace_depth -= 1;
        }
        if brace_depth > 0 {
            body_end += 1;
        }
    }

    let body = s[body_start..body_end].to_string();
    Some((selector, body, body_end + 1))
}

/// Extract unscoped rules from CSS text.
///
/// These are rules that should apply under the polyfill but NOT be scoped.
///
/// Converts polyfill-unscoped-rule rules like:
/// ```css
/// @polyfill-unscoped-rule {
///   content: 'menu-item';
///   background: blue;
/// }
/// ```
///
/// to:
/// ```css
/// menu-item { background: blue; }
/// ```
///
/// Returns the extracted unscoped rules as a string.
fn extract_unscoped_rules(css: &str) -> String {
    let mut result = String::new();
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Case-insensitive check for "polyfill-unscoped-rule"
        // Only check if we're at a valid char boundary to avoid panics with UTF-8
        if i + 22 <= len
            && css.is_char_boundary(i)
            && css.is_char_boundary(i + 22)
            && css[i..i + 22].eq_ignore_ascii_case("polyfill-unscoped-rule")
        {
            if let Some((selector, body, skip_to)) = parse_polyfill_unscoped_rule(&css[i..]) {
                let cleaned_body = body.trim().trim_start_matches(';').trim();
                result.push_str(&selector);
                result.push_str(" { ");
                result.push_str(cleaned_body);
                result.push_str(" }\n\n");
                i += skip_to;
                continue;
            }
        }
        i += 1;
    }

    result
}

/// Parse a polyfill-unscoped-rule block and return (selector, body, chars_to_skip).
/// The pattern is: polyfill-unscoped-rule { content: 'selector'; ...body... }
fn parse_polyfill_unscoped_rule(s: &str) -> Option<(String, String, usize)> {
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Skip "polyfill-unscoped-rule"
    let mut i = 22;

    // Find the opening brace
    while i < len && bytes[i] != b'{' {
        i += 1;
    }
    if i >= len {
        return None;
    }
    i += 1; // skip {

    // Find "content:" (case-insensitive)
    let mut content_found = false;
    let mut selector = String::new();
    let mut body_start = i;

    while i + 8 <= len {
        if s[i..].to_ascii_lowercase().starts_with("content") {
            let after_content = i + 7;
            let mut j = after_content;
            while j < len && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < len && bytes[j] == b':' {
                j += 1;
                while j < len && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }

                if j < len && (bytes[j] == b'\'' || bytes[j] == b'"') {
                    let quote = bytes[j];
                    let content_start = j + 1;
                    let mut content_end = content_start;
                    while content_end < len && bytes[content_end] != quote {
                        content_end += 1;
                    }
                    selector = s[content_start..content_end].to_string();
                    content_found = true;

                    // Skip past the closing quote and any semicolon/whitespace
                    body_start = content_end + 1;
                    while body_start < len
                        && (bytes[body_start].is_ascii_whitespace() || bytes[body_start] == b';')
                    {
                        body_start += 1;
                    }
                    break;
                }
            }
        }
        i += 1;
    }

    if !content_found {
        return None;
    }

    // Find the closing brace
    let mut body_end = body_start;
    let mut brace_depth = 1;
    while body_end < len && brace_depth > 0 {
        if bytes[body_end] == b'{' {
            brace_depth += 1;
        } else if bytes[body_end] == b'}' {
            brace_depth -= 1;
        }
        if brace_depth > 0 {
            body_end += 1;
        }
    }

    let body = s[body_start..body_end].to_string();
    Some((selector, body, body_end + 1))
}

/// Remove polyfill-unscoped-rule blocks from CSS text.
///
/// This is called after extracting the unscoped rules.
fn remove_unscoped_rules(css: &str) -> String {
    let mut result = String::with_capacity(css.len());
    let bytes = css.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Case-insensitive check for "polyfill-unscoped-rule"
        // Only check if we're at a valid char boundary to avoid panics with UTF-8
        if i + 22 <= len
            && css.is_char_boundary(i)
            && css.is_char_boundary(i + 22)
            && css[i..i + 22].eq_ignore_ascii_case("polyfill-unscoped-rule")
        {
            // Skip the entire block
            if let Some(skip_to) = skip_polyfill_unscoped_rule(&css[i..]) {
                i += skip_to;
                continue;
            }
        }
        i += push_utf8_char(&mut result, css, i);
    }

    result
}

/// Skip over a polyfill-unscoped-rule block and return chars to skip.
fn skip_polyfill_unscoped_rule(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let len = bytes.len();

    // Skip "polyfill-unscoped-rule"
    let mut i = 22;

    // Find the opening brace
    while i < len && bytes[i] != b'{' {
        i += 1;
    }
    if i >= len {
        return None;
    }
    i += 1; // skip {

    // Find the matching closing brace
    let mut brace_depth = 1;
    while i < len && brace_depth > 0 {
        if bytes[i] == b'{' {
            brace_depth += 1;
        } else if bytes[i] == b'}' {
            brace_depth -= 1;
        }
        i += 1;
    }

    Some(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The forward scan [`closes_enclosing_paren_table`] replaces.
    fn closes_enclosing_paren_scan(s: &str, from: usize) -> bool {
        let mut depth = 0u32;
        for &b in &s.as_bytes()[from..] {
            match b {
                b'(' => {
                    depth += 1;
                    if depth > 3 {
                        return false;
                    }
                }
                b')' => {
                    if depth == 0 {
                        return true;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        false
    }

    #[test]
    fn test_closes_enclosing_paren_table_matches_forward_scan() {
        // Every string of up to 8 bytes over `(`, `)`, `a`.
        let alphabet = *b"()a";
        for len in 0..=8u32 {
            for mut code in 0..3usize.pow(len) {
                let mut bytes = Vec::with_capacity(len as usize);
                for _ in 0..len {
                    bytes.push(alphabet[code % 3]);
                    code /= 3;
                }
                let s = String::from_utf8(bytes).unwrap();
                let table = closes_enclosing_paren_table(&s);
                assert_eq!(table.len(), s.len() + 1);
                for (from, &closes) in table.iter().enumerate() {
                    assert_eq!(closes, closes_enclosing_paren_scan(&s, from), "{s:?} at {from}");
                }
            }
        }
    }

    #[test]
    fn test_basic_encapsulation() {
        let result = shim_css_text(".button { color: red; }", "contenta", "");
        assert!(result.contains(".button[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_multiple_selectors() {
        let result = shim_css_text("h1, h2, h3 { font-weight: bold; }", "contenta", "");
        assert!(result.contains("h1[contenta]"), "Got: {}", result);
        assert!(result.contains("h2[contenta]"), "Got: {}", result);
        assert!(result.contains("h3[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_descendant_selectors() {
        let result = shim_css_text("one two { color: red; }", "contenta", "");
        assert!(result.contains("one[contenta]"), "Got: {}", result);
        assert!(result.contains("two[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_child_selectors() {
        let result = shim_css_text("one > two { color: red; }", "contenta", "");
        assert!(result.contains("one[contenta]"), "Got: {}", result);
        assert!(result.contains("two[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_host_selector() {
        let result = shim_css_text(":host { display: block; }", "contenta", "a-host");
        assert!(result.contains("[a-host]"), "Got: {}", result);
        assert!(!result.contains(":host"), "Got: {}", result);
    }

    #[test]
    fn test_host_with_class() {
        let result = shim_css_text(":host(.active) { opacity: 1; }", "contenta", "a-host");
        assert!(result.contains(".active[a-host]"), "Got: {}", result);
    }

    #[test]
    fn test_ng_deep() {
        let result = shim_css_text("::ng-deep .child { color: blue; }", "contenta", "");
        assert!(!result.contains("::ng-deep"), "Got: {}", result);
        assert!(result.contains(".child"), "Got: {}", result);
    }

    #[test]
    fn test_pseudo_elements() {
        let result = shim_css_text(".button::before { content: ''; }", "contenta", "");
        assert!(result.contains(".button[contenta]::before"), "Got: {}", result);
    }

    #[test]
    fn test_media_query() {
        let result =
            shim_css_text("@media (max-width: 600px) { .button { color: red; } }", "contenta", "");
        assert!(result.contains("@media"), "Got: {}", result);
        assert!(result.contains(".button[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_strip_scoping_selectors() {
        // Test the strip function directly
        // `::ng-deep` becomes a space; the space after it and the block stay.
        let result = strip_scoping_selectors(" ::ng-deep @top-left { content: \"Hamlet\";}");
        assert_eq!(result, "   @top-left { content: \"Hamlet\";}");
    }

    #[test]
    fn test_page_strip_ng_deep() {
        let result =
            shim_css_text("@page { ::ng-deep @top-left { content: \"Hamlet\";}}", "contenta", "h");
        // Expected: "@page { @top-left { content:\"Hamlet\";}}" (with space after @page {)
        assert!(!result.contains("::ng-deep"), "Got: {}", result);
        assert!(result.contains("@top-left"), "Got: {}", result);
        assert!(result.contains("\"Hamlet\""), "Got: {}", result);
        // Check for space after @page {
        assert!(result.starts_with("@page { "), "Expected space after '@page {{', got: {}", result);
    }

    // ============================================================================
    // Polyfill Directive Tests
    // ============================================================================

    /// The polyfill passes only run for Angular < v22.0.0-rc.0.
    fn shim_v21(css: &str, content_attr: &str, host_attr: &str) -> String {
        shim_css_text_for_version(css, content_attr, host_attr, Some(AngularVersion::new(21, 2, 0)))
    }

    #[test]
    fn test_polyfill_next_selector_single_quotes() {
        let result = shim_v21("polyfill-next-selector {content: 'x > y'} z {}", "contenta", "");
        assert!(result.contains("x[contenta]"), "Got: {}", result);
        assert!(result.contains("y[contenta]"), "Got: {}", result);
        assert!(!result.contains("polyfill-next-selector"), "Got: {}", result);
        assert!(!result.contains("z"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_next_selector_double_quotes() {
        let result = shim_v21("polyfill-next-selector {content: \"x > y\"} z {}", "contenta", "");
        assert!(result.contains("x[contenta]"), "Got: {}", result);
        assert!(result.contains("y[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_next_selector_with_attribute() {
        let result = shim_v21(
            "polyfill-next-selector {content: 'button[priority=\"1\"]'} z {}",
            "contenta",
            "",
        );
        assert!(result.contains("button[priority=\"1\"][contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_unscoped_rule_single_quotes() {
        let result = shim_v21(
            "polyfill-unscoped-rule {content: '#menu > .bar';color: blue;}",
            "contenta",
            "",
        );
        // Unscoped rules should NOT have the content attribute
        assert!(result.contains("#menu > .bar"), "Got: {}", result);
        assert!(result.contains("color: blue"), "Got: {}", result);
        assert!(
            !result.contains("[contenta]") || result.contains("#menu > .bar {"),
            "Unscoped rule should not have content attribute. Got: {}",
            result
        );
    }

    #[test]
    fn test_polyfill_unscoped_rule_double_quotes() {
        let result = shim_v21(
            "polyfill-unscoped-rule {content: \"#menu > .bar\";color: blue;}",
            "contenta",
            "",
        );
        assert!(result.contains("#menu > .bar"), "Got: {}", result);
        assert!(result.contains("color: blue"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_unscoped_rule_multiple() {
        let result = shim_v21(
            "polyfill-unscoped-rule {content: 'foo';color: blue;}polyfill-unscoped-rule {content: 'bar';color: red;}",
            "contenta",
            "",
        );
        assert!(result.contains("foo {"), "Got: {}", result);
        assert!(result.contains("bar {"), "Got: {}", result);
        assert!(result.contains("color: blue"), "Got: {}", result);
        assert!(result.contains("color: red"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_rule_single_quotes() {
        let result = shim_v21(
            "polyfill-rule {content: ':host.foo .bar';color: blue;}",
            "contenta",
            "a-host",
        );
        // polyfill-rule content gets scoped like a regular rule
        assert!(result.contains(".foo[a-host]"), "Got: {}", result);
        assert!(result.contains(".bar[contenta]"), "Got: {}", result);
        assert!(result.contains("color: blue"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_rule_double_quotes() {
        let result = shim_v21(
            "polyfill-rule {content: \":host.foo .bar\";color: blue;}",
            "contenta",
            "a-host",
        );
        assert!(result.contains(".foo[a-host]"), "Got: {}", result);
        assert!(result.contains(".bar[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_rule_with_attribute() {
        let result =
            shim_v21("polyfill-rule {content: 'button[priority=\"1\"]'}", "contenta", "a-host");
        assert!(result.contains("button[priority=\"1\"][contenta]"), "Got: {}", result);
    }

    // ============================================================================

    #[test]
    fn test_polyfills_are_plain_selectors_in_v22() {
        // Angular v22 dropped the polyfill passes: the blocks are scoped like
        // any other rule and `::shadow` / `::content` are kept.
        assert_eq!(
            shim_css_text("polyfill-next-selector {content: 'x > y'} z {}", "contenta", ""),
            "polyfill-next-selector[contenta] {content: 'x > y'} z[contenta] {}"
        );
        assert_eq!(
            shim_css_text(
                "polyfill-unscoped-rule {content: '#menu > .bar';color: blue;}",
                "contenta",
                ""
            ),
            "polyfill-unscoped-rule[contenta] {content: '#menu > .bar';color: blue;}"
        );
        assert_eq!(
            shim_css_text("x::content y {}", "contenta", ""),
            "x[contenta]::content   y[contenta] {}"
        );
        assert_eq!(shim_v21("x::content y {}", "contenta", ""), "x[contenta]   y[contenta] {}");
    }

    // ============================================================================
    // Regression test for exponential :host-context explosion
    // See: https://github.com/nickreese/oxc/issues/XXX
    // ============================================================================

    #[test]
    fn test_host_context_no_exponential_explosion() {
        // This CSS has multiple separate :host-context rules.
        // Each rule should be processed independently, NOT multiplied together.
        // Previously this caused 350MB output and hangs due to exponential permutation.
        let css = r#"
.row-actions {
  :host-context(.menu-opened-hierarchy-item) &,
  :host-context(.expanded) &,
  :host-context(.tree-node:hover) &,
  :host-context(.everything-row) & {
    visibility: visible;
  }
  :host-context(.sidebar-category-row_editor) &,
  :host-context(.sidebar-category-row_editor:hover) &,
  :host-context(.sidebar-category-row_editor .expanded) & {
    visibility: hidden;
  }
}
"#;
        let result = shim_css_text(css, "_ngcontent-%COMP%", "_nghost-%COMP%");

        // The output should be reasonably sized (not exponential)
        // With proper processing, this should be < 50KB, not 350MB
        assert!(
            result.len() < 100_000,
            "Output should not explode exponentially. Got {} bytes",
            result.len()
        );
    }

    #[test]
    fn test_nested_host_context_reasonable_output() {
        // Nested :host-context inside another :host-context rule
        let css = r#"
.foo {
  :host-context(.a) &,
  :host-context(.b) &,
  :host-context(.c) &,
  :host-context(.d) & {
    color: red;
    :host-context(.e) & {
      color: blue;
    }
    :host-context(.f) & {
      color: green;
    }
  }
  :host-context(.g) &,
  :host-context(.h) &,
  :host-context(.i) & {
    color: yellow;
  }
}
"#;
        let result = shim_css_text(css, "_ngcontent-%COMP%", "_nghost-%COMP%");

        // Output should be reasonable, not 500KB+ from exponential multiplication
        assert!(
            result.len() < 50_000,
            "Output should not explode exponentially. Got {} bytes",
            result.len()
        );
    }

    #[test]
    fn test_multibyte_utf8_in_selector() {
        // Selectors with multibyte UTF-8 characters (e.g. attribute selectors with
        // non-ASCII values) must not panic from byte/char index mismatch.
        let result = shim_css_text(r#"[data-label="ÄÖÜ"] .child { color: red; }"#, "contenta", "");
        assert!(
            result.contains("[contenta]"),
            "Should scope selectors containing multibyte UTF-8. Got: {}",
            result
        );
    }

    #[test]
    fn test_multibyte_utf8_pseudo_element() {
        // Pseudo-elements on selectors with multibyte characters must not panic.
        let result = shim_css_text(r#"[title="café"]::before { content: ""; }"#, "contenta", "");
        assert!(
            result.contains("[contenta]"),
            "Should scope pseudo-elements with multibyte UTF-8. Got: {}",
            result
        );
    }

    #[test]
    fn test_multibyte_utf8_pseudo_class() {
        // Pseudo-classes on selectors with multibyte characters must not panic.
        let result = shim_css_text(r#".naïve:hover { color: blue; }"#, "contenta", "");
        assert!(
            result.contains("[contenta]"),
            "Should scope pseudo-classes with multibyte UTF-8. Got: {}",
            result
        );
    }

    #[test]
    fn test_multibyte_utf8_combinator_split() {
        // Combinators between selectors with multibyte characters must not panic.
        let result =
            shim_css_text(r#".über > .straße + .café ~ .naïve { color: green; }"#, "contenta", "");
        assert!(
            result.contains("[contenta]"),
            "Should handle combinators with multibyte UTF-8 selectors. Got: {}",
            result
        );
    }
}

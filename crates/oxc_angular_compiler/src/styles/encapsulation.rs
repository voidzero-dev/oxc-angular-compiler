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

/// Placeholder for comments during processing.
const COMMENT_PLACEHOLDER: &str = "%COMMENT%";

// Polyfill host markers (matching Angular's shadow_css.ts)
const POLYFILL_HOST: &str = "-shadowcsshost";
const POLYFILL_HOST_NO_COMBINATOR: &str = "-shadowcsshost-no-combinator";

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

/// SafeSelector escapes problematic patterns before selector processing
/// and restores them after. This prevents attribute selector contents,
/// escaped characters, and :nth-*() expressions from being misinterpreted.
///
/// Port of Angular's `SafeSelector` class from shadow_css.ts.
struct SafeSelector {
    placeholders: Vec<String>,
    content: String,
}

impl SafeSelector {
    /// Creates a new SafeSelector, escaping problematic patterns.
    fn new(selector: &str) -> Self {
        let mut placeholders = Vec::new();
        let mut index = 0;
        let mut result = selector.to_string();

        // 1. Escape attribute selectors WITH VALUES: [attr="va lue"] -> __ph-N__
        // This prevents spaces inside attribute values from being treated as combinators.
        // We only escape selectors with values (=, ~=, |=, ^=, $=, *=) because:
        // - Simple attribute presence [attr] doesn't contain problematic characters
        // - Our generated selectors like [hosta] and [contenta] need to remain visible
        //   for host detection in contains_host_attr_at_top_level()
        {
            let mut new_result = String::new();
            let bytes = result.as_bytes();
            let len = bytes.len();
            let mut i = 0;
            while i < len {
                if bytes[i] == b'[' {
                    // Found start of attribute selector
                    let attr_start = i;
                    let mut j = i + 1;
                    let mut has_value_op = false;

                    // Scan until we find ] or end of string
                    while j < len && bytes[j] != b']' {
                        // Check for value operators: =, ~=, |=, ^=, $=, *=
                        if bytes[j] == b'=' {
                            has_value_op = true;
                        } else if j + 1 < len
                            && bytes[j + 1] == b'='
                            && matches!(bytes[j], b'~' | b'|' | b'^' | b'$' | b'*')
                        {
                            has_value_op = true;
                        }
                        j += 1;
                    }

                    if j < len && bytes[j] == b']' && has_value_op {
                        // Found attribute selector with value - escape it
                        let attr_end = j + 1;
                        let attr_selector = &result[attr_start..attr_end];
                        let placeholder = format!("__ph-{}__", index);
                        placeholders.push(attr_selector.to_string());
                        index += 1;
                        new_result.push_str(&placeholder);
                        i = attr_end;
                    } else {
                        // Simple attribute selector without value - keep as-is
                        new_result.push('[');
                        i += 1;
                    }
                } else {
                    i += push_utf8_char(&mut new_result, &result, i);
                }
            }
            result = new_result;
        }

        // 2. Escape backslash sequences: \: -> __esc-ph-N__
        // This handles escaped special characters (e.g., .foo\:bar for class "foo:bar")
        {
            let mut new_result = String::new();
            let bytes = result.as_bytes();
            let len = bytes.len();
            let mut i = 0;
            while i < len {
                if bytes[i] == b'\\' && i + 1 < len {
                    // Found escape sequence - capture both backslash and next char
                    let placeholder = format!("__esc-ph-{}__", index);
                    let escape_seq = &result[i..i + 2];
                    placeholders.push(escape_seq.to_string());
                    index += 1;
                    new_result.push_str(&placeholder);
                    i += 2;
                } else {
                    i += push_utf8_char(&mut new_result, &result, i);
                }
            }
            result = new_result;
        }

        // 3. Escape :nth-*() expressions: :nth-child(2n + 1) -> :nth-child__ph-N__
        // This prevents + and spaces inside nth expressions from being treated as combinators.
        // We need to handle nested parens like :nth-child(3n of :not(p, a), :is(.foo))
        // Use manual parsing to find matching closing paren (handles any nesting level).
        let mut new_result = String::new();
        let mut chars_iter = result.char_indices().peekable();

        while let Some((i, c)) = chars_iter.next() {
            // Look for :nth-* pattern
            if c == ':' {
                let remaining = &result[i..];
                if remaining.starts_with(":nth-") {
                    // Find the pseudo name (e.g., :nth-child, :nth-of-type)
                    let pseudo_end = remaining[5..]
                        .find(|c: char| !c.is_alphanumeric() && c != '-')
                        .map(|pos| 5 + pos)
                        .unwrap_or(remaining.len());

                    let pseudo = &remaining[..pseudo_end];

                    // Check if followed by (
                    if remaining[pseudo_end..].starts_with('(') {
                        let paren_start = i + pseudo_end;
                        // Find matching closing paren
                        let mut depth = 1;
                        let mut paren_end = paren_start + 1;
                        for (j, ch) in result[paren_start + 1..].char_indices() {
                            match ch {
                                '(' => depth += 1,
                                ')' => {
                                    depth -= 1;
                                    if depth == 0 {
                                        paren_end = paren_start + 1 + j;
                                        break;
                                    }
                                }
                                _ => {}
                            }
                        }

                        if depth == 0 {
                            // Found matching paren - escape the content
                            let content = &result[paren_start + 1..paren_end];
                            let placeholder = format!("__ph-{}__", index);
                            placeholders.push(format!("({})", content));
                            index += 1;

                            new_result.push_str(pseudo);
                            new_result.push_str(&placeholder);

                            // Skip past the processed characters
                            while let Some(&(idx, _)) = chars_iter.peek() {
                                if idx <= paren_end {
                                    chars_iter.next();
                                } else {
                                    break;
                                }
                            }
                            continue;
                        }
                    }
                }
            }
            new_result.push(c);
        }
        result = new_result;

        SafeSelector { placeholders, content: result }
    }

    /// Returns the escaped content for processing.
    fn content(&self) -> &str {
        &self.content
    }

    /// Restores the original patterns from placeholders.
    fn restore(&self, content: &str) -> String {
        let mut result = content.to_string();

        // Restore placeholders in reverse order to avoid index shifting issues
        // Replace __ph-N__ patterns
        for (idx, replacement) in self.placeholders.iter().enumerate().rev() {
            let placeholder = format!("__ph-{}__", idx);
            result = result.replace(&placeholder, replacement);
        }

        // Replace __esc-ph-N__ patterns
        for (idx, replacement) in self.placeholders.iter().enumerate().rev() {
            let placeholder = format!("__esc-ph-{}__", idx);
            result = result.replace(&placeholder, replacement);
        }

        result
    }
}

/// Shim CSS text with the given selectors.
///
/// This is the main entry point matching Angular's `ShadowCss.shimCssText()`.
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
    if css.is_empty() {
        return String::new();
    }

    // Step 0: Extract comments and replace with placeholders
    // This prevents comment contents from interfering with CSS parsing
    let (result, comments) = extract_comments(css);

    let mut result = result;

    // Step 1: Process polyfill directives (polyfill-next-selector, polyfill-rule)
    // These convert ShadowDOM rules to work with the CSS shim
    result = insert_polyfill_directives(&result);
    result = insert_polyfill_rules(&result);

    // Step 2: Extract unscoped rules (polyfill-unscoped-rule)
    // These rules are added at the end WITHOUT scoping
    let unscoped_rules = extract_unscoped_rules(&result);
    result = remove_unscoped_rules(&result);

    // Step 3: Handle :host-context() - must be done before :host
    result = convert_colon_host_context(&result, content_attr, host_attr);

    // Step 4: Handle :host selectors
    result = convert_colon_host(&result, host_attr);

    // Step 5: Handle ::ng-deep and other shadow DOM selectors
    result = convert_shadow_dom_selectors(&result);

    // Step 6: Scope keyframes and animation properties
    if !content_attr.is_empty() {
        result = scope_keyframes_related_css(&result, content_attr);
    }

    // Step 7: Scope all selectors with the content attribute
    if !content_attr.is_empty() {
        result = scope_selectors(&result, content_attr, host_attr);
    }

    // Step 8: Append unscoped rules (without scoping)
    if !unscoped_rules.is_empty() {
        result = result + "\n" + unscoped_rules.trim();
    }

    // Step 9: Restore comments
    restore_comments(&result, &comments)
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

            if is_sourcemap {
                comments.push(comment.to_string());
            } else {
                // Count newlines in the comment to preserve line count for sourcemaps
                let newline_count = comment.bytes().filter(|&b| b == b'\n').count();
                let mut preserved = String::new();
                for _ in 0..newline_count {
                    preserved.push('\n');
                }
                preserved.push('\n');
                comments.push(preserved);
            }

            result.push_str(COMMENT_PLACEHOLDER);
        } else {
            i += push_utf8_char(&mut result, css, i);
        }
    }

    (result, comments)
}

/// Restore comments from placeholders.
fn restore_comments(css: &str, comments: &[String]) -> String {
    let mut result = css.to_string();
    let mut idx = 0;

    while result.find(COMMENT_PLACEHOLDER).is_some() {
        if idx < comments.len() {
            result = result.replacen(COMMENT_PLACEHOLDER, &comments[idx], 1);
            idx += 1;
        } else {
            break;
        }
    }

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
    let content_attr = format!("ng-c{}", component_id);
    let host_attr = format!("ng-c{}", component_id);
    shim_css_text(css, &content_attr, &host_attr)
}

// =============================================================================
// Host selector processing
// =============================================================================

/// Convert :host-context() selectors with permutation generation.
///
/// For a single `:host-context(.dark)`:
///   - `.dark[host_attr], .dark [host_attr]`
///
/// For `:host-context(.one,.two)` (comma-separated inside):
///   - `.one[host_attr], .one [host_attr], .two[host_attr], .two [host_attr]`
///
/// For multiple `:host-context(.a):host-context(.b)`:
///   - Generates all permutations: `.a.b`, `.a .b`, `.b .a`
///   - Each with host variants: `perm[host]` and `perm [host]`
///
/// When otherSelectors contains `:host`, only one variant per permutation is generated.
///
/// See: packages/compiler/src/shadow_css.ts `_combineHostContextSelectors`
fn convert_colon_host_context(css: &str, content_attr: &str, host_attr: &str) -> String {
    // IMPORTANT: We must process :host-context only within each CSS rule's selector,
    // not across the entire CSS. Otherwise, multiple :host-context rules will be
    // combined exponentially, causing massive output explosion.
    //
    // Strategy: Scan through CSS, identify each selector (text before {), process
    // :host-context only within that selector, then continue to the next rule.

    let mut result = String::with_capacity(css.len() * 2);
    let mut chars = css.chars().peekable();
    let mut current_selector = String::new();
    let mut brace_depth: u32 = 0;
    let mut in_string = false;
    let mut string_char = '"';

    while let Some(c) = chars.next() {
        // Handle string literals
        if !in_string && (c == '"' || c == '\'') {
            in_string = true;
            string_char = c;
            if brace_depth == 0 {
                current_selector.push(c);
            } else {
                result.push(c);
            }
            continue;
        }
        if in_string {
            if brace_depth == 0 {
                current_selector.push(c);
            } else {
                result.push(c);
            }
            if c == string_char {
                in_string = false;
            }
            continue;
        }

        match c {
            '{' => {
                if brace_depth == 0 {
                    // End of selector - process :host-context in this selector only
                    let processed_selector = convert_colon_host_context_in_selector(
                        &current_selector,
                        content_attr,
                        host_attr,
                    );
                    result.push_str(&processed_selector);
                    current_selector.clear();
                }
                result.push('{');
                brace_depth += 1;
            }
            '}' => {
                result.push('}');
                brace_depth = brace_depth.saturating_sub(1);
            }
            _ => {
                if brace_depth == 0 {
                    current_selector.push(c);
                } else {
                    result.push(c);
                }
            }
        }
    }

    // Handle any remaining selector content
    if !current_selector.is_empty() {
        let processed_selector =
            convert_colon_host_context_in_selector(&current_selector, content_attr, host_attr);
        result.push_str(&processed_selector);
    }

    result
}

/// Process :host-context within a single CSS selector (before the `{`).
/// This is called once per CSS rule, preventing exponential multiplication across rules.
fn convert_colon_host_context_in_selector(
    selector: &str,
    content_attr: &str,
    host_attr: &str,
) -> String {
    // Split by top-level commas to handle selector lists
    // (commas not inside parentheses)
    let parts: Vec<&str> = split_by_top_level_comma_str(selector);

    let results: Vec<String> = parts
        .iter()
        .map(|part| convert_colon_host_context_in_part(part, content_attr, host_attr))
        .collect();

    results.join(",")
}

/// Split a string by top-level commas (not inside parentheses or braces).
fn split_by_top_level_comma_str(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut paren_depth: u32 = 0;
    let mut brace_depth: u32 = 0;

    for (i, c) in s.char_indices() {
        match c {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '{' => brace_depth += 1,
            '}' => brace_depth = brace_depth.saturating_sub(1),
            ',' if paren_depth == 0 && brace_depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// Process :host-context in a single selector part.
fn convert_colon_host_context_in_part(css: &str, content_attr: &str, host_attr: &str) -> String {
    let attr = if host_attr.is_empty() {
        format!("[{}]", content_attr)
    } else {
        format!("[{}]", host_attr)
    };

    // Find the first :host-context
    let host_context_marker = ":host-context";
    let Some(hc_global_start) = css.find(host_context_marker) else {
        return css.to_string();
    };

    // Check if there's a :where() or :is() immediately before the :host-context
    // Pattern: something:where(:host-context...) or :where(:host-context...)
    let before_hc = &css[..hc_global_start];
    let (pseudo_prefix, prefix_start) = find_pseudo_function_before(before_hc);

    // Everything before the pseudo-function (or before :host-context if no pseudo)
    let preserved_prefix = &css[..prefix_start];

    // The part to process starts from prefix_start
    let to_process = &css[prefix_start..];

    // Find where the replacement region starts in to_process
    let hc_local_start = to_process.find(host_context_marker).unwrap_or(0);

    // Build context selector groups by processing all :host-context instances
    let mut selector_groups: Vec<Vec<String>> = vec![vec![]];
    let mut current_pos = hc_local_start;
    let to_process_bytes = to_process.as_bytes();

    while current_pos < to_process.len() {
        // Find next :host-context
        let Some(hc_start) = to_process[current_pos..].find(host_context_marker) else {
            break;
        };
        let hc_start = current_pos + hc_start;

        let after_marker = hc_start + host_context_marker.len();

        // Check if followed by (
        if after_marker >= to_process.len() || to_process_bytes[after_marker] != b'(' {
            // :host-context with no parens (edge case)
            // Just skip this :host-context marker and move on
            current_pos = after_marker;
            continue;
        }

        // Find matching closing paren using balanced matching
        let paren_start = after_marker + 1;
        let Some(paren_end) = find_matching_paren(to_process, paren_start) else {
            break; // Unbalanced
        };

        // Extract the content inside parentheses
        let inner_content = &to_process[paren_start..paren_end];

        // Split by top-level commas (for :host-context(.a, .b))
        let comma_separated: Vec<String> = split_by_top_level_comma_str(inner_content)
            .iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        if !comma_separated.is_empty() {
            // Duplicate groups for cartesian product
            let original_length = selector_groups.len();
            for _ in 1..comma_separated.len() {
                for i in 0..original_length {
                    selector_groups.push(selector_groups[i].clone());
                }
            }

            // Add each selector to the appropriate group
            for (i, selector) in comma_separated.iter().enumerate() {
                for j in 0..original_length {
                    selector_groups[j + i * original_length].push(selector.clone());
                }
            }
        }

        current_pos = paren_end + 1;
    }

    // Handle edge cases: :host-context with no selectors
    // e.g., ":host-context .inner" or ":host-context() .inner"
    if selector_groups.is_empty() || selector_groups[0].is_empty() {
        // Remove the :host-context (with or without empty parens) and replace with host marker
        let result = replace_host_context_patterns(css, &attr);
        return result;
    }

    // The "other selectors" are everything after the last :host-context()
    // Stop at '{' to not include the declaration block
    let remaining_after = &to_process[current_pos..];
    let other_end = remaining_after.find('{').unwrap_or(remaining_after.len());
    let other_selectors = &remaining_after[..other_end];

    // Check if :host appears in other_selectors (before :host conversion)
    let other_has_host = other_selectors.contains(":host");

    // Generate combined selectors
    let results: Vec<String> = selector_groups
        .iter()
        .flat_map(|group| {
            combine_host_context_selectors_with_other(
                group,
                other_selectors,
                &pseudo_prefix,
                &attr,
                other_has_host,
            )
        })
        .collect();

    // Append the part after other_selectors (the { and everything after)
    let suffix = &remaining_after[other_end..];
    format!("{}{}{}", preserved_prefix, results.join(", "), suffix)
}

/// Find :where() or :is() immediately before :host-context.
/// Returns (pseudo_prefix like ":where(", position where it starts).
fn find_pseudo_function_before(before_hc: &str) -> (String, usize) {
    // Check if ends with :where( or :is(
    for prefix in &[":where(", ":is("] {
        if before_hc.ends_with(prefix) {
            let start = before_hc.len() - prefix.len();
            return (prefix.to_string(), start);
        }
    }
    (String::new(), before_hc.len())
}

/// Find the matching closing parenthesis, handling nested parens.
/// Returns the index of the closing paren (exclusive).
fn find_matching_paren(s: &str, start: usize) -> Option<usize> {
    let mut depth = 1;
    let chars: Vec<char> = s[start..].chars().collect();

    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(start + i);
                }
            }
            _ => {}
        }
    }
    None
}

/// Combine host-context selectors with other selectors, respecting the Angular algorithm.
fn combine_host_context_selectors_with_other(
    context_selectors: &[String],
    other_selectors: &str,
    pseudo_prefix: &str,
    host_attr: &str,
    other_has_host: bool,
) -> Vec<String> {
    if context_selectors.is_empty() {
        // No context selectors - just return host marker + other
        return vec![format!("{}{}{}", pseudo_prefix, host_attr, other_selectors)];
    }

    // Generate permutations (compound, ancestor, descendant forms)
    let permutations = combine_host_context_selectors(context_selectors);

    permutations
        .iter()
        .flat_map(|perm| {
            if other_has_host {
                // If other selectors contain :host, only one variant
                vec![format!("{}{}{}", pseudo_prefix, perm, other_selectors)]
            } else {
                // Two variants: compound (selector[host]) and descendant (selector [host])
                // Note: if pseudo_prefix is set, other_selectors already includes the closing paren
                vec![
                    format!("{}{}{}{}", pseudo_prefix, perm, host_attr, other_selectors),
                    format!("{}{} {}{}", pseudo_prefix, perm, host_attr, other_selectors),
                ]
            }
        })
        .collect()
}

/// Combine multiple host-context selectors into all permutations.
///
/// For selectors [".a", ".b"], generates:
///   - ".a.b" (compound)
///   - ".a .b" (.a ancestor)
///   - ".b .a" (.b ancestor)
///
/// For selectors [".a", ".b", ".c"], generates 9 permutations.
///
/// See: packages/compiler/src/shadow_css.ts `_combineHostContextSelectors`
fn combine_host_context_selectors(context_selectors: &[String]) -> Vec<String> {
    if context_selectors.is_empty() {
        return vec![];
    }

    if context_selectors.len() == 1 {
        return vec![context_selectors[0].clone()];
    }

    // Work from right to left as the TypeScript implementation does
    let mut selectors = context_selectors.to_vec();
    // Safety: We checked len >= 2 above, so pop() will succeed
    let Some(last) = selectors.pop() else {
        return vec![];
    };
    let mut combined: Vec<String> = vec![last];

    while let Some(context_selector) = selectors.pop() {
        let length = combined.len();
        let mut new_combined: Vec<String> = Vec::with_capacity(length * 3);

        for i in 0..length {
            let previous = &combined[i];

            // Compound form (no space): contextSelector + previousSelectors
            new_combined.push(format!("{}{}", context_selector, previous));

            // Ancestor form (with space): contextSelector + ' ' + previousSelectors
            new_combined.push(format!("{} {}", context_selector, previous));

            // Descendant form (with space): previousSelectors + ' ' + contextSelector
            new_combined.push(format!("{} {}", previous, context_selector));
        }

        combined = new_combined;
    }

    combined
}

/// Convert :host selectors.
///
/// `:host` becomes `[host_attr]`
/// `:host(.active)` becomes `.active[host_attr]`
/// Converts `:host` selectors to use polyfill markers.
///
/// This function implements Angular's marker-based approach:
/// 1. Replace `:host` with `POLYFILL_HOST` marker
/// 2. Process `POLYFILL_HOST(...)` patterns (direct parens)
/// 3. Process `POLYFILL_HOST` followed by selectors until comma/brace
///
/// The actual conversion from markers to host attributes happens later
/// in `scope_selectors`, after the selector is split by combinators.
///
/// Key insight: matching stops at COMMAS, not closing parens of pseudo-functions.
/// This ensures `:host:not(:host.foo, :host.bar)` is processed correctly.
fn convert_colon_host(css: &str, _host_attr: &str) -> String {
    // Step 1: Replace all :host with marker
    let mut result = css.replace(":host", POLYFILL_HOST);

    // Step 2: Process POLYFILL_HOST(...) patterns - direct parens after marker
    result = process_host_with_parens(&result);

    // Step 3: Process POLYFILL_HOST followed by selectors until comma/brace
    // This uses global matching that stops at commas (like Angular's _cssColonHostRe)
    result = process_host_with_selectors(&result);

    // Note: Marker-to-attr conversion happens in scope_selectors, not here.
    // This is because Angular does the conversion AFTER splitting by combinators.

    result
}

/// Process POLYFILL_HOST(...) patterns where parens immediately follow the marker.
fn process_host_with_parens(css: &str) -> String {
    let mut result = css.to_string();
    let pattern = format!("{}(", POLYFILL_HOST);

    loop {
        let Some(host_start) = result.find(&pattern) else {
            break;
        };

        let paren_start = host_start + pattern.len();

        // Find matching closing paren
        let mut paren_depth = 1;
        let mut paren_end = paren_start;
        for (i, c) in result[paren_start..].char_indices() {
            match c {
                '(' => paren_depth += 1,
                ')' => {
                    paren_depth -= 1;
                    if paren_depth == 0 {
                        paren_end = paren_start + i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if paren_depth != 0 {
            break; // Unbalanced parens
        }

        let inner_content = &result[paren_start..paren_end];
        let after_host = &result[paren_end + 1..];

        // Get other selectors until comma or brace
        let other_end = after_host.find(|c: char| c == '{' || c == ',').unwrap_or(after_host.len());
        let other_selectors = &after_host[..other_end];

        // Split inner by top-level commas
        let inner_selectors = split_by_top_level_comma(inner_content);

        // Process each inner selector
        let converted: Vec<String> = inner_selectors
            .iter()
            .map(|sel| {
                let trimmed = sel.trim();
                if trimmed.is_empty() {
                    return String::new();
                }
                // Use POLYFILL_HOST_NO_COMBINATOR + trimmed (with marker stripped) + other
                let stripped = trimmed.replace(POLYFILL_HOST, "");
                format!("{}{}{}", POLYFILL_HOST_NO_COMBINATOR, stripped, other_selectors)
            })
            .filter(|s| !s.is_empty())
            .collect();

        let replacement = converted.join(",");
        result = format!("{}{}{}", &result[..host_start], replacement, &after_host[other_end..]);
    }

    result
}

/// Process POLYFILL_HOST followed by selectors until comma/brace.
/// This matches Angular's _cssColonHostRe behavior with global matching.
fn process_host_with_selectors(css: &str) -> String {
    let mut result = String::with_capacity(css.len() * 2);
    let mut remaining = css;

    while !remaining.is_empty() {
        // Find next POLYFILL_HOST
        let Some(host_pos) = remaining.find(POLYFILL_HOST) else {
            result.push_str(remaining);
            break;
        };

        // Add everything before the marker
        result.push_str(&remaining[..host_pos]);
        remaining = &remaining[host_pos..];

        // Check if this is already POLYFILL_HOST_NO_COMBINATOR
        if remaining.starts_with(POLYFILL_HOST_NO_COMBINATOR) {
            // Already processed, keep as-is
            result.push_str(POLYFILL_HOST_NO_COMBINATOR);
            remaining = &remaining[POLYFILL_HOST_NO_COMBINATOR.len()..];
            continue;
        }

        // Skip the POLYFILL_HOST marker
        remaining = &remaining[POLYFILL_HOST.len()..];

        // Check if followed by ( - already handled by process_host_with_parens
        if remaining.starts_with('(') {
            // This shouldn't happen if process_host_with_parens ran first,
            // but handle it just in case by keeping the marker
            result.push_str(POLYFILL_HOST);
            continue;
        }

        // Find selectors until comma or brace (Angular's [^,{]* pattern)
        let mut selector_end = 0;
        for (i, c) in remaining.char_indices() {
            if c == ',' || c == '{' {
                break;
            }
            selector_end = i + c.len_utf8();
        }

        let other_selectors = &remaining[..selector_end];
        remaining = &remaining[selector_end..];

        // Convert to POLYFILL_HOST_NO_COMBINATOR + selectors
        result.push_str(POLYFILL_HOST_NO_COMBINATOR);
        result.push_str(other_selectors);
    }

    result
}

/// Split a string by top-level commas (not inside parentheses).
fn split_by_top_level_comma(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut paren_depth: u32 = 0;

    for (i, c) in s.char_indices() {
        match c {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ',' if paren_depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    result.push(&s[start..]);
    result
}

/// Reorder a selector to put the host attribute before any pseudo-selectors.
/// Uses the pattern ([^:\)]*)(:*)(.*)
/// Examples:
/// - ":before" -> "[attr]:before"
/// - ".active" -> ".active[attr]"
/// - ".active:before" -> ".active[attr]:before"
/// - ":not(.x)" -> "[attr]:not(.x)"
/// Reorders a selector to insert the host attribute at the correct position.
/// Matches Angular's pattern: /([^:\)]*)(:*)(.*)/
/// - before: everything before first colon or close paren
/// - colon: any colons at that position
/// - after: everything else
fn reorder_selector_with_attr(selector: &str, attr: &str) -> String {
    if attr.is_empty() {
        return selector.to_string();
    }

    // Find the first colon or close paren that's not inside brackets
    let mut bracket_depth: u32 = 0;
    let mut split_pos = None;
    let chars: Vec<char> = selector.chars().collect();

    for (i, c) in chars.iter().enumerate() {
        match c {
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ':' | ')' if bracket_depth == 0 => {
                split_pos = Some(i);
                break;
            }
            _ => {}
        }
    }

    if let Some(pos) = split_pos {
        let before: String = chars[..pos].iter().collect();
        let after: String = chars[pos..].iter().collect();
        format!("{}{}{}", before, attr, after)
    } else {
        format!("{}{}", selector, attr)
    }
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

/// At-rules that should NOT have scoping selectors applied (e.g., @font-face, @page).
/// These rules define global resources that shouldn't be component-scoped.
/// All other at-rules (@media, @supports, @layer, @container, etc.) have their contents scoped.
const NO_SCOPE_AT_RULES: &[&str] = &["@font-face", "@page"];

/// Scope all CSS selectors with the content attribute.
fn scope_selectors(css: &str, content_attr: &str, host_attr: &str) -> String {
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
    let mut at_rule_header = String::new(); // Collect the at-rule header text
    let mut in_no_scope_at_rule = false; // Track if we're inside @font-face/@page
    let mut no_scope_depth: u32 = 0; // Track nesting depth for no-scope at-rules
    let mut no_scope_content = String::new(); // Collect content for no-scope at-rules

    while let Some(c) = chars.next() {
        // Handle comments
        if !in_string && c == '/' && chars.peek() == Some(&'*') {
            in_comment = true;
            if in_at_rule_header || in_declaration {
                result.push(c);
            } else {
                current_selector.push(c);
            }
            continue;
        }
        if in_comment {
            if in_at_rule_header || in_declaration {
                result.push(c);
            } else {
                current_selector.push(c);
            }
            if c == '*' && chars.peek() == Some(&'/') {
                if let Some(next_c) = chars.next() {
                    if in_at_rule_header || in_declaration {
                        result.push(next_c);
                    } else {
                        current_selector.push(next_c);
                    }
                }
                in_comment = false;
            }
            continue;
        }

        // Handle strings
        if !in_string && (c == '"' || c == '\'') {
            in_string = true;
            string_char = c;
            if in_no_scope_at_rule {
                no_scope_content.push(c);
            } else if in_at_rule_header || in_declaration {
                result.push(c);
            } else {
                current_selector.push(c);
            }
            continue;
        }
        if in_string {
            if in_no_scope_at_rule {
                no_scope_content.push(c);
            } else if in_at_rule_header || in_declaration {
                result.push(c);
            } else {
                current_selector.push(c);
            }
            if c == string_char {
                in_string = false;
            }
            continue;
        }

        // Handle content inside @font-face/@page (no scoping, but strip selectors)
        // This must be checked BEFORE the @ rule handling so nested @-rules
        // (like @top-left inside @page) are collected properly
        if in_no_scope_at_rule {
            if c == '{' {
                no_scope_depth += 1;
                no_scope_content.push(c);
            } else if c == '}' {
                no_scope_depth -= 1;
                if no_scope_depth == 0 {
                    // Strip ::ng-deep, >>>, /deep/, and :host from the collected content
                    let stripped = strip_scoping_selectors(&no_scope_content, host_attr);
                    result.push_str(&stripped);
                    result.push('}');
                    no_scope_content.clear();
                    in_no_scope_at_rule = false;
                    at_rule_depth = at_rule_depth.saturating_sub(1);
                } else {
                    no_scope_content.push(c);
                }
            } else {
                no_scope_content.push(c);
            }
            continue;
        }

        // Handle @ rules (media queries, keyframes, etc.)
        if c == '@' {
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

                // Check if this is a no-scope at-rule (@font-face, @page)
                let header_lower = at_rule_header.to_lowercase();
                if NO_SCOPE_AT_RULES.iter().any(|r| header_lower.starts_with(&r.to_lowercase())) {
                    in_no_scope_at_rule = true;
                    no_scope_depth = 1;
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
            at_rule_depth = at_rule_depth.saturating_sub(1);
            result.push(c);
            continue;
        }

        // If inside a declaration block, just copy content directly
        if in_declaration {
            result.push(c);
            if c == '{' {
                declaration_depth += 1;
            } else if c == '}' {
                declaration_depth -= 1;
                if declaration_depth == 0 {
                    in_declaration = false;
                }
            }
            continue;
        }

        // Regular selector parsing
        if c == '{' {
            // End of selector, scope it
            let scoped = scope_selector_list(&current_selector, content_attr, host_attr);
            result.push_str(&scoped);
            result.push('{');
            current_selector.clear();
            in_declaration = true;
            declaration_depth = 1;
        } else if c == '}' {
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
// Scoping Context - Implements Angular's _shouldScopeIndicator pattern
// =============================================================================

/// Context for tracking scoping state across recursive calls.
/// This implements Angular's `_shouldScopeIndicator` pattern.
///
/// The key insight is:
/// - Selectors BEFORE :host should NOT be scoped (they match ancestor elements)
/// - Selectors AFTER :host SHOULD be scoped (they match component descendants)
/// - This state must persist across recursive calls into pseudo-functions
#[derive(Clone)]
struct ScopingContext<'a> {
    content_attr: &'a str,
    host_attr: &'a str,
    /// The host marker string, e.g., "[hosta]"
    host_marker: String,
    /// Whether selector parts should be scoped.
    /// - None: not yet determined
    /// - Some(false): don't scope (before :host)
    /// - Some(true): do scope (after :host or no :host in selector)
    should_scope_indicator: Option<bool>,
    /// Whether this is the parent (top-level) selector call
    is_parent_selector: bool,
}

impl<'a> ScopingContext<'a> {
    fn new(content_attr: &'a str, host_attr: &'a str, is_parent: bool) -> Self {
        let host_marker =
            if host_attr.is_empty() { String::new() } else { format!("[{}]", host_attr) };
        Self {
            content_attr,
            host_attr,
            host_marker,
            should_scope_indicator: None,
            is_parent_selector: is_parent,
        }
    }

    /// Create a child context for recursive calls (not a parent selector)
    fn child(&self) -> Self {
        Self {
            content_attr: self.content_attr,
            host_attr: self.host_attr,
            host_marker: self.host_marker.clone(),
            should_scope_indicator: self.should_scope_indicator,
            is_parent_selector: false,
        }
    }

    /// Check if a selector contains the host marker (either the actual attr or polyfill markers)
    fn contains_host(&self, selector: &str) -> bool {
        // Check for actual host attribute marker like [a-host]
        let has_attr_marker = !self.host_marker.is_empty() && selector.contains(&self.host_marker);
        // Also check for polyfill markers (-shadowcsshost or -shadowcsshost-no-combinator)
        let has_polyfill_marker = selector.contains(POLYFILL_HOST);
        has_attr_marker || has_polyfill_marker
    }

    /// Initialize or update the scoping indicator based on Angular's logic:
    /// if (isParentSelector || this._shouldScopeIndicator) {
    ///   this._shouldScopeIndicator = !hasHost;
    /// }
    /// Note: In JavaScript, `_shouldScopeIndicator` uses truthiness:
    /// - undefined → falsy
    /// - false → falsy
    /// - true → truthy
    /// So we only reinitialize if parent OR indicator is already true (not just Some).
    fn initialize_scoping(&mut self, has_host: bool) {
        if self.is_parent_selector || self.should_scope_indicator == Some(true) {
            self.should_scope_indicator = Some(!has_host);
        }
    }

    /// Update the indicator when we encounter :host in a part
    /// this._shouldScopeIndicator = this._shouldScopeIndicator || partContainsHost
    fn update_on_host_found(&mut self) {
        self.should_scope_indicator = Some(true);
    }

    /// Check if we should scope a part
    fn should_scope(&self) -> bool {
        self.should_scope_indicator.unwrap_or(true)
    }

    /// Check if a selector contains the -no-combinator marker
    fn contains_polyfill_host_no_combinator(&self, selector: &str) -> bool {
        selector.contains(POLYFILL_HOST_NO_COMBINATOR)
    }

    /// Convert polyfill host markers to actual host attributes.
    /// This implements Angular's _applySimpleSelectorScope logic.
    fn apply_simple_selector_scope(&self, selector: &str) -> String {
        // Note: This function should ONLY be called when the selector contains
        // polyfill host markers. The caller must check contains_polyfill_host first.

        let mut result = selector.to_string();

        // Process -shadowcsshost-no-combinator patterns with reordering
        loop {
            let Some(marker_pos) = result.find(POLYFILL_HOST_NO_COMBINATOR) else {
                break;
            };

            let after_marker = &result[marker_pos + POLYFILL_HOST_NO_COMBINATOR.len()..];

            // Find end of attached selector (until whitespace or comma)
            let mut selector_end = 0;
            for (i, c) in after_marker.char_indices() {
                if c.is_whitespace() || c == ',' {
                    break;
                }
                selector_end = i + c.len_utf8();
            }

            let attached_selector = &after_marker[..selector_end];

            // Apply reordering: before + host_attr + after
            let converted = reorder_selector_with_attr(attached_selector, &self.host_marker);

            result =
                format!("{}{}{}", &result[..marker_pos], converted, &after_marker[selector_end..]);
        }

        // Replace remaining POLYFILL_HOST with host_marker
        result.replace(POLYFILL_HOST, &self.host_marker)
    }
}

/// Scope a comma-separated list of selectors.
fn scope_selector_list(selector_list: &str, content_attr: &str, host_attr: &str) -> String {
    // Create the parent context for this top-level call
    let ctx = ScopingContext::new(content_attr, host_attr, true);
    scope_selector_list_with_context(selector_list, &ctx)
}

/// Scope a selector list with a given context (allows recursive calls to share state)
fn scope_selector_list_with_context(selector_list: &str, parent_ctx: &ScopingContext) -> String {
    // Preserve leading and trailing whitespace
    let leading: &str = &selector_list[..selector_list.len() - selector_list.trim_start().len()];
    let trimmed = selector_list.trim();
    if trimmed.is_empty() {
        return selector_list.to_string();
    }
    let trailing: &str = &selector_list[selector_list.trim_end().len()..];

    // Create SafeSelector to escape problematic patterns
    // (attribute selectors, escaped characters, :nth-*() expressions)
    let safe_selector = SafeSelector::new(trimmed);
    let safe_content = safe_selector.content();

    // Split by comma (respecting parentheses)
    let selectors = split_by_comma(safe_content);
    let scoped: Vec<String> = selectors
        .iter()
        .map(|s| {
            // Each comma-separated selector gets its own context copy
            let mut ctx = parent_ctx.clone();
            scope_complex_selector_with_context(s.trim(), &mut ctx)
        })
        .collect();

    let scoped_result = scoped.join(", ");

    // Restore escaped patterns
    let restored = safe_selector.restore(&scoped_result);

    format!("{}{}{}", leading, restored, trailing)
}

/// Split a string by top-level commas (not inside parentheses).
fn split_by_comma(s: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut paren_depth: u32 = 0;

    for (i, c) in s.char_indices() {
        match c {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            ',' if paren_depth == 0 => {
                result.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }

    result.push(&s[start..]);
    result
}

/// Scope a complex selector (may contain combinators).
/// Scope a complex selector (may contain combinators).
///
/// For `one two > three`, we scope each simple selector: `one[attr] two[attr] > three[attr]`
///
/// ::ng-deep, >>>, /deep/ are handled specially: parts AFTER these deep combinators
/// are NOT scoped - they "escape" the encapsulation.
fn scope_complex_selector_with_context(selector: &str, ctx: &mut ScopingContext) -> String {
    if selector.is_empty() {
        return String::new();
    }

    // Don't scope keyframe selectors
    if is_keyframe_selector(selector) {
        return selector.to_string();
    }

    // Split by deep combinators (::ng-deep, >>>, /deep/)
    // Parts after the deep combinator are NOT scoped
    let deep_parts = split_by_deep_combinators(selector);

    let shallow_part = deep_parts[0];
    let other_parts = &deep_parts[1..];

    // Initialize scoping based on whether the shallow part contains :host
    let has_host = ctx.contains_host(shallow_part);
    ctx.initialize_scoping(has_host);

    // Scope only the shallow part (before ::ng-deep)
    let scoped_shallow = scope_shallow_selector_with_context(shallow_part, ctx);

    // Join with the unscoped other parts (after ::ng-deep becomes a space)
    if other_parts.is_empty() {
        scoped_shallow
    } else {
        format!("{} {}", scoped_shallow, other_parts.join(" "))
    }
}

/// Scope a shallow selector with context tracking.
/// Implements Angular's _scopeSelector logic with _shouldScopeIndicator.
fn scope_shallow_selector_with_context(selector: &str, ctx: &mut ScopingContext) -> String {
    if selector.is_empty() {
        return String::new();
    }

    // Split by combinators while preserving them
    let parts = split_by_combinators(selector);

    let mut result = String::new();
    for (part, combinator) in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            if !combinator.is_empty() {
                result.push_str(&normalize_combinator(combinator));
            }
            continue;
        }

        // Check if this part contains the host marker
        let part_has_host = ctx.contains_host(trimmed);

        // Update the scoping indicator: once we see :host, subsequent parts should be scoped
        // Angular: this._shouldScopeIndicator = this._shouldScopeIndicator || partContainsHost
        // IMPORTANT: Angular only updates for parts INSIDE the loop (parts with combinators after).
        // The final part (after the loop) does NOT get this update - the pseudo-function handler
        // updates internally as it processes each pseudo-function.
        if part_has_host && !combinator.is_empty() {
            ctx.update_on_host_found();
        }

        // Scope the part based on the indicator
        let scoped = scope_selector_part_with_context(trimmed, ctx, part_has_host);

        result.push_str(&scoped);
        if !combinator.is_empty() {
            result.push_str(&normalize_combinator_with_context(combinator, trimmed));
        }
    }

    result
}

/// Normalize combinator spacing, taking into account hex escape sequences.
/// When the preceding part ends with a hex escape (escape placeholder + hex digits),
/// we need to output extra spaces to preserve the hex escape terminator.
fn normalize_combinator_with_context(combinator: &str, preceding_part: &str) -> String {
    let trimmed = combinator.trim();
    if trimmed.is_empty() {
        // Just whitespace - check if we need extra spaces for hex escape termination
        if ends_with_hex_escape_pattern(preceding_part) {
            // Output 3 spaces: original escape terminator + combinator space + extra for clarity
            "   ".to_string()
        } else {
            " ".to_string()
        }
    } else {
        // Non-space combinator (>, +, ~) - add spaces around it
        format!(" {} ", trimmed)
    }
}

/// Check if a selector part ends with a hex escape pattern.
/// This detects patterns like `__esc-ph-N__` followed by hex digits.
fn ends_with_hex_escape_pattern(s: &str) -> bool {
    // Look for __esc-ph-N__ followed by optional hex digits at the end
    if let Some(esc_pos) = s.rfind("__esc-ph-") {
        // Find the end of the placeholder (the closing __)
        let after_start = esc_pos + 9; // length of "__esc-ph-"
        if let Some(rel_end) = s[after_start..].find("__") {
            let placeholder_end = after_start + rel_end + 2;
            // Check if remaining chars (after placeholder) are all hex digits
            let remaining = &s[placeholder_end..];
            !remaining.is_empty() && remaining.chars().all(|c| c.is_ascii_hexdigit())
        } else {
            false
        }
    } else {
        false
    }
}

/// Scope a single selector part, respecting the context's should_scope indicator.
fn scope_selector_part_with_context(
    selector: &str,
    ctx: &mut ScopingContext,
    part_has_host: bool,
) -> String {
    if selector.is_empty() {
        return String::new();
    }

    // If this part IS the host marker, don't add content attr
    if !ctx.host_marker.is_empty() && selector.trim() == ctx.host_marker {
        return selector.to_string();
    }

    // Check if this is a pseudo-function (:where, :is)
    // These need special handling - we may need to scope their contents
    if let Some(scoped) = try_scope_pseudo_function_with_context(selector, ctx) {
        return scoped;
    }

    // If the selector is NOT purely pseudo-functions but contains host,
    // update the indicator. This matches Angular's else branch in
    // _pseudoFunctionAwareScopeSelectorPart.
    if part_has_host {
        ctx.update_on_host_found();
    }

    // Handle polyfill host markers (Angular's _scopeSelectorPart with host markers)
    if ctx.contains_polyfill_host_no_combinator(selector) {
        let mut scoped = ctx.apply_simple_selector_scope(selector);

        // If marker is inside a pseudo-function (not outside), also add content attr
        // Angular uses: if (!p.match(_polyfillHostNoCombinatorOutsidePseudoFunction))
        // which matches if the marker is NOT followed by [^(]*\) - i.e., IS inside parens
        if !is_marker_outside_pseudo_function(selector) && !ctx.content_attr.is_empty() {
            // Add content attr using the reorder pattern
            scoped = scope_simple_selector(&scoped, ctx.content_attr);
        }
        return scoped;
    }

    // For parts containing the host marker at top level but with other content,
    // we need to scope the parts after the host
    if part_has_host && contains_host_attr_at_top_level(selector, ctx.host_attr) {
        return scope_after_host_with_context(selector, ctx);
    }

    // Only scope if the indicator says we should
    if !ctx.should_scope() {
        return selector.to_string();
    }

    // Apply the content attribute
    scope_simple_selector(selector, ctx.content_attr)
}

/// Check if the polyfill host marker is outside any pseudo-function parentheses.
/// Returns true if marker is at top level (not inside parens).
fn is_marker_outside_pseudo_function(selector: &str) -> bool {
    // Find the marker position
    let Some(marker_pos) = selector.find(POLYFILL_HOST_NO_COMBINATOR) else {
        return false;
    };

    // Check if after the marker, there's a closing paren before an opening paren
    // Angular regex: (?![^(]*\)) - negative lookahead for [^(]*\)
    // This means "not followed by (any non-open-paren chars) then close-paren"
    let after_marker = &selector[marker_pos + POLYFILL_HOST_NO_COMBINATOR.len()..];

    // If we find ) before ( (or no paren at all), marker is inside parens -> return false
    // If we find ( before ) (or only () pairs), marker is outside -> return true
    let mut paren_depth = 0;
    for c in after_marker.chars() {
        match c {
            '(' => {
                paren_depth += 1;
            }
            ')' => {
                if paren_depth == 0 {
                    // Found closing paren without matching open -> marker is inside parens
                    return false;
                }
                paren_depth -= 1;
            }
            _ => {}
        }
    }
    // No unmatched closing paren found -> marker is outside parens
    true
}

/// Try to scope pseudo-functions (:where, :is, :has, :not) with context tracking.
/// Returns None if the selector has other parts before the pseudo-function.
///
/// This is the context-aware version that implements Angular's `_pseudoFunctionAwareScopeSelectorPart`.
fn try_scope_pseudo_function_with_context(
    selector: &str,
    ctx: &mut ScopingContext,
) -> Option<String> {
    let trimmed = selector.trim();

    // Collect all outer :where() and :is() selectors only.
    // Angular only scopes inside these pseudo-functions, not :has() or :not().
    // For :has() and :not(), the content attr goes BEFORE the pseudo-function.

    // Find all pseudo-function parts
    let mut pseudo_parts: Vec<String> = Vec::new();
    let mut last_end = 0;
    let chars: Vec<char> = trimmed.chars().collect();

    let mut search_from = 0;
    while let Some(mat) = find_where_or_is(trimmed, search_from) {
        let match_start = mat.start;

        // If there's content before this pseudo-function that's not another pseudo-function,
        // this selector has other parts
        if match_start > last_end {
            let between = &trimmed[last_end..match_start];
            if !between.is_empty() {
                // There's content between pseudo-functions - not a pure pseudo-function selector
                return None;
            }
        }

        // Find the matching closing paren
        let paren_start = mat.end;
        let mut paren_depth = 1;
        let mut paren_end = paren_start;

        for i in paren_start..trimmed.len() {
            match chars[i] {
                '(' => paren_depth += 1,
                ')' => {
                    paren_depth -= 1;
                    if paren_depth == 0 {
                        paren_end = i;
                        break;
                    }
                }
                _ => {}
            }
        }

        if paren_depth != 0 {
            return None; // Unbalanced parens
        }

        // Capture the full pseudo-function including content
        pseudo_parts.push(trimmed[match_start..=paren_end].to_string());
        last_end = paren_end + 1;
        search_from = last_end;
    }

    // Check if the entire selector is made up of pseudo-functions
    if pseudo_parts.is_empty() {
        return None;
    }

    let joined: String = pseudo_parts.join("");
    if joined != trimmed {
        return None; // Selector has other parts
    }

    // Process each pseudo-function
    let scoped_parts: Vec<String> = pseudo_parts
        .iter()
        .map(|part| {
            // Extract the pseudo-function name and inner content
            let Some(pm) = find_where_or_is(part, 0) else {
                return part.clone();
            };

            let prefix_without_paren = &part[..pm.end - 1]; // e.g., ":where"
            let inner = &part[pm.end..part.len() - 1]; // Content inside parens

            // Check if inner content contains host marker - update context if so
            if ctx.contains_host(inner) {
                ctx.update_on_host_found();
            }

            // Create a child context for recursive scoping
            let child_ctx = ctx.child();

            // Recursively scope the inner content
            let scoped_inner = scope_selector_list_with_context(inner, &child_ctx);

            // Update parent context from child if host was found
            if child_ctx.should_scope_indicator == Some(true) {
                ctx.update_on_host_found();
            }

            format!("{}({})", prefix_without_paren, scoped_inner)
        })
        .collect();

    Some(scoped_parts.join(""))
}

/// Scope a simple selector by adding the content attribute.
fn scope_simple_selector(selector: &str, content_attr: &str) -> String {
    if selector.is_empty() {
        return String::new();
    }

    // Don't scope comment placeholders
    if selector.contains(COMMENT_PLACEHOLDER) {
        return selector.to_string();
    }

    // Already has the content attribute
    let attr = format!("[{}]", content_attr);
    if selector.contains(&attr) {
        return selector.to_string();
    }

    // Don't scope keyframe selectors
    if is_keyframe_selector(selector) {
        return selector.to_string();
    }

    // Handle pseudo-elements first (::before, ::after, etc.)
    if let Some(pseudo_pos) = find_pseudo_element_start(selector) {
        let (base, pseudo) = selector.split_at(pseudo_pos);
        return format!("{}{}{}", base, attr, pseudo);
    }

    // Handle pseudo-classes (:hover, :first-child, etc.)
    if let Some(pseudo_pos) = find_pseudo_class_start(selector) {
        let (base, pseudo) = selector.split_at(pseudo_pos);
        if base.is_empty() {
            return format!("{}{}", attr, selector);
        }
        return format!("{}{}{}", base, attr, pseudo);
    }

    format!("{}{}", selector, attr)
}

/// Normalize combinator spacing to match Angular's format.
/// Angular outputs ` separator ` for all combinators.
fn normalize_combinator(combinator: &str) -> String {
    let trimmed = combinator.trim();
    if trimmed.is_empty() {
        // Just whitespace - return single space
        " ".to_string()
    } else {
        // Non-space combinator (>, +, ~) - add spaces around it
        format!(" {} ", trimmed)
    }
}

/// Check if the host attr appears at the top level (not inside parentheses).
fn contains_host_attr_at_top_level(selector: &str, host_attr: &str) -> bool {
    let host_marker = format!("[{}]", host_attr);
    let mut depth: u32 = 0;
    let chars: Vec<char> = selector.chars().collect();
    let marker_chars: Vec<char> = host_marker.chars().collect();

    for i in 0..chars.len() {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            _ if depth == 0 && i + marker_chars.len() <= chars.len() => {
                // Check if marker starts at this position
                if chars[i..i + marker_chars.len()] == marker_chars[..] {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Split a selector by combinators (space, >, +, ~).
/// Returns pairs of (selector_part, combinator_with_spaces).
fn split_by_combinators(selector: &str) -> Vec<(&str, &str)> {
    let mut result = Vec::new();
    let chars: Vec<char> = selector.chars().collect();
    let mut start = 0;
    let mut i = 0;
    let mut paren_depth: u32 = 0;
    let mut bracket_depth: u32 = 0;

    while i < chars.len() {
        match chars[i] {
            '(' => paren_depth += 1,
            ')' => paren_depth = paren_depth.saturating_sub(1),
            '[' => bracket_depth += 1,
            ']' => bracket_depth = bracket_depth.saturating_sub(1),
            ' ' | '\n' | '\t' | '\r' | '>' | '+' | '~'
                if paren_depth == 0 && bracket_depth == 0 =>
            {
                // A space following an escaped hex value and followed by another hex character
                // (ie: ".\fc ber" for ".über") is not a separator between 2 selectors
                // Check: if the part ends with an escape placeholder AND next char is hex
                let part = &selector[start..i];
                let next_char_is_hex =
                    i + 1 < chars.len() && chars[i] == ' ' && chars[i + 1].is_ascii_hexdigit();
                let part_ends_with_esc_placeholder = part.contains("__esc-ph-");

                if next_char_is_hex && part_ends_with_esc_placeholder {
                    // This space is part of a CSS hex escape sequence, not a combinator
                    i += 1;
                    continue;
                }

                // Found a potential combinator
                let part_end = i;

                // Collect the combinator (may include spaces around it)
                let combinator_start = i;
                while i < chars.len()
                    && (chars[i] == ' '
                        || chars[i] == '\n'
                        || chars[i] == '\t'
                        || chars[i] == '\r'
                        || chars[i] == '>'
                        || chars[i] == '+'
                        || chars[i] == '~')
                {
                    i += 1;
                }

                // Always push the part, even if empty (to preserve leading combinators)
                result.push((&selector[start..part_end], &selector[combinator_start..i]));
                start = i;
                continue;
            }
            _ => {}
        }
        i += 1;
    }

    // Add the last part
    if start < selector.len() {
        result.push((&selector[start..], ""));
    }

    result
}

/// Split a selector by deep combinators (`>>>`, `/deep/`, `::ng-deep`).
/// Returns the parts between the deep combinators.
/// Parts after the first deep combinator are not scoped.
fn split_by_deep_combinators(selector: &str) -> Vec<&str> {
    let mut result = Vec::new();
    let bytes = selector.as_bytes();
    let len = bytes.len();
    let mut start = 0;
    let mut i = 0;

    while i < len {
        // Check for `>>>` (3 chars)
        if i + 3 <= len
            && selector.is_char_boundary(i)
            && selector.is_char_boundary(i + 3)
            && &selector[i..i + 3] == ">>>"
        {
            result.push(&selector[start..i]);
            i += 3;
            // Skip trailing whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }

        // Check for `/deep/` (6 chars)
        if i + 6 <= len
            && selector.is_char_boundary(i)
            && selector.is_char_boundary(i + 6)
            && &selector[i..i + 6] == "/deep/"
        {
            result.push(&selector[start..i]);
            i += 6;
            // Skip trailing whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }

        // Check for `::ng-deep` (9 chars)
        if i + 9 <= len
            && selector.is_char_boundary(i)
            && selector.is_char_boundary(i + 9)
            && &selector[i..i + 9] == "::ng-deep"
        {
            result.push(&selector[start..i]);
            i += 9;
            // Skip trailing whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            start = i;
            continue;
        }

        i += 1;
    }

    // Add the remaining part
    if start <= len {
        result.push(&selector[start..]);
    }

    // If nothing was split, return the original selector as the only element
    if result.is_empty() {
        result.push(selector);
    }

    result
}

/// Scope descendants after :host in a selector with context.
fn scope_after_host_with_context(selector: &str, ctx: &mut ScopingContext) -> String {
    if ctx.host_marker.is_empty() {
        return selector.to_string();
    }

    if let Some(pos) = selector.find(&ctx.host_marker) {
        let after_host = &selector[pos + ctx.host_marker.len()..];
        let before_host = &selector[..pos];
        let before_and_host = &selector[..pos + ctx.host_marker.len()];

        // Parts before :host should NOT be scoped
        // Parts after :host SHOULD be scoped
        // So we mark that we've seen host
        ctx.update_on_host_found();

        // Check if what follows is a pseudo-selector directly attached (no combinator)
        let after_trimmed = after_host.trim_start();
        if after_trimmed.is_empty() {
            // Handle any content before :host - it should NOT be scoped
            if before_host.is_empty() {
                return selector.to_string();
            }
            // Return before (unscoped) + host
            return selector.to_string();
        }

        // If after_host starts with a pseudo-selector (: but not preceded by a combinator),
        // it's directly attached to the host and shouldn't be scoped
        let first_char = after_trimmed.chars().next();
        if first_char == Some(':') {
            // Check if there's a combinator somewhere after the pseudo-selector
            let parts = split_by_combinators(after_trimmed);
            if parts.len() == 1 && parts[0].1.is_empty() {
                // Only one part with no combinator - it's all pseudo-selectors on the host
                return selector.to_string();
            }

            // There are combinators - scope everything after the first combinator
            let mut scoped_after = String::new();
            let mut found_combinator = false;
            for (part, combinator) in parts {
                if !found_combinator {
                    // First part (pseudo-selector attached to host) - don't scope
                    scoped_after.push_str(part);
                    if !combinator.is_empty()
                        && combinator.chars().any(|c| {
                            c == ' '
                                || c == '\n'
                                || c == '\t'
                                || c == '\r'
                                || c == '>'
                                || c == '+'
                                || c == '~'
                        })
                    {
                        found_combinator = true;
                    }
                    scoped_after.push_str(&normalize_combinator_with_context(combinator, part));
                } else {
                    // Parts after combinator - scope them
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        scoped_after.push_str(&scope_simple_selector(trimmed, ctx.content_attr));
                    }
                    if !combinator.is_empty() {
                        scoped_after.push_str(&normalize_combinator_with_context(combinator, part));
                    }
                }
            }
            return format!("{}{}", before_and_host, scoped_after);
        }

        // Normal case: what follows is not a pseudo-selector, scope it
        let parts = split_by_combinators(after_host);
        let mut scoped_after = String::new();
        for (part, combinator) in parts {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                scoped_after.push_str(&scope_simple_selector(trimmed, ctx.content_attr));
            }
            if !combinator.is_empty() {
                scoped_after.push_str(&normalize_combinator_with_context(combinator, part));
            }
        }

        format!("{}{}", before_and_host, scoped_after)
    } else {
        selector.to_string()
    }
}

/// Find the start position of a pseudo-element (::).
fn find_pseudo_element_start(s: &str) -> Option<usize> {
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut in_brackets: u32 = 0;

    while i < chars.len() {
        match chars[i] {
            '[' => in_brackets += 1,
            ']' => in_brackets = in_brackets.saturating_sub(1),
            ':' if in_brackets == 0 && i + 1 < chars.len() && chars[i + 1] == ':' => {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Find the start position of a pseudo-class (:), including pseudo-functions.
/// The caller decides how to handle pseudo-functions vs regular pseudo-classes.
fn find_pseudo_class_start(s: &str) -> Option<usize> {
    let mut i = 0;
    let chars: Vec<char> = s.chars().collect();
    let mut in_brackets: u32 = 0;

    while i < chars.len() {
        match chars[i] {
            '[' => in_brackets += 1,
            ']' => in_brackets = in_brackets.saturating_sub(1),
            ':' if in_brackets == 0 => {
                // Check it's not :: (pseudo-element) - those are handled separately
                if i + 1 < chars.len() && chars[i + 1] == ':' {
                    return None;
                }
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Result of finding a pseudo-function match.
struct PseudoFunctionMatch {
    /// Start position of the match (at the `:`)
    start: usize,
    /// End position of the match (after the `(`)
    end: usize,
}

/// Find `:where(` or `:is(` pattern starting from a given position.
/// Returns the match info if found, or None.
fn find_where_or_is(s: &str, start_from: usize) -> Option<PseudoFunctionMatch> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = start_from;

    while i < len {
        if bytes[i] == b':' {
            // Check for `:where(`
            if i + 7 <= len && &s[i..i + 7] == ":where(" {
                return Some(PseudoFunctionMatch { start: i, end: i + 7 });
            }
            // Check for `:is(`
            if i + 4 <= len && &s[i..i + 4] == ":is(" {
                return Some(PseudoFunctionMatch { start: i, end: i + 4 });
            }
        }
        i += 1;
    }
    None
}

/// Replace `:host-context` patterns (with or without empty parens) with a replacement string.
/// Matches `:host-context` or `:host-context()` (with optional whitespace inside parens).
fn replace_host_context_patterns(s: &str, replacement: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for `:host-context`
        if i + 13 <= len && &s[i..i + 13] == ":host-context" {
            let after = i + 13;
            // Check if followed by `(` with optional whitespace and `)`
            if after < len && bytes[after] == b'(' {
                // Skip whitespace inside parens
                let mut j = after + 1;
                while j < len && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                if j < len && bytes[j] == b')' {
                    // Found :host-context() - replace entire thing
                    result.push_str(replacement);
                    i = j + 1;
                    continue;
                }
            }
            // Just :host-context without () or with non-empty ()
            result.push_str(replacement);
            i = after;
            continue;
        }
        i += push_utf8_char(&mut result, s, i);
    }

    result
}

/// Check if a selector is a keyframe selector (from, to, or percentage).
fn is_keyframe_selector(selector: &str) -> bool {
    let trimmed = selector.trim();
    if trimmed == "from" || trimmed == "to" {
        return true;
    }
    // Check for percentage (e.g., "50%", "100%")
    if trimmed.ends_with('%') {
        let without_percent = &trimmed[..trimmed.len() - 1];
        return without_percent.parse::<f64>().is_ok();
    }
    false
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

/// Strip scoping selectors (::ng-deep, >>>, /deep/, :host) from CSS text.
///
/// This is used for @font-face and @page rules where these selectors should
/// be removed entirely rather than scoped.
fn strip_scoping_selectors(css: &str, host_attr: &str) -> String {
    // Strip ::ng-deep, >>>, /deep/
    let result = strip_deep_combinators(css);

    // Strip polyfill host markers (-shadowcsshost-no-combinator and -shadowcsshost)
    // The -no-combinator variant must be replaced first since it's longer
    let mut result = result.replace(POLYFILL_HOST_NO_COMBINATOR, " ").replace(POLYFILL_HOST, " ");

    // Strip :host (with optional selector in parens)
    // If host_attr is provided, :host may have already been converted to [host_attr]
    if !host_attr.is_empty() {
        let host_marker = format!("[{}]", host_attr);
        result = result.replace(&host_marker, " ");
    }

    // Also strip unconverted :host patterns
    result = strip_host_patterns(&result);

    // Clean up multiple spaces
    collapse_multiple_spaces(&result)
}

/// Strip deep combinators (::ng-deep, >>>, /deep/) from a string, replacing with space.
fn strip_deep_combinators(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for `::ng-deep`
        if i + 9 <= len && &s[i..i + 9] == "::ng-deep" {
            result.push(' ');
            i += 9;
            // Skip trailing whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }

        // Check for `>>>`
        if i + 3 <= len && &s[i..i + 3] == ">>>" {
            result.push(' ');
            i += 3;
            // Skip trailing whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }

        // Check for `/deep/`
        if i + 6 <= len && &s[i..i + 6] == "/deep/" {
            result.push(' ');
            i += 6;
            // Skip trailing whitespace
            while i < len && bytes[i].is_ascii_whitespace() {
                i += 1;
            }
            continue;
        }

        i += push_utf8_char(&mut result, s, i);
    }

    result
}

/// Strip :host patterns, including :host() with optional content in parens.
fn strip_host_patterns(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Check for `:host`
        if i + 5 <= len && &s[i..i + 5] == ":host" {
            let after = i + 5;
            // Check if followed by `(`
            if after < len && bytes[after] == b'(' {
                // Find matching close paren
                let mut paren_depth = 1;
                let mut j = after + 1;
                while j < len && paren_depth > 0 {
                    if bytes[j] == b'(' {
                        paren_depth += 1;
                    } else if bytes[j] == b')' {
                        paren_depth -= 1;
                    }
                    j += 1;
                }
                result.push(' ');
                i = j;
                // Skip trailing whitespace
                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            } else {
                // Just `:host` without parens
                result.push(' ');
                i = after;
                // Skip trailing whitespace
                while i < len && bytes[i].is_ascii_whitespace() {
                    i += 1;
                }
            }
            continue;
        }

        i += push_utf8_char(&mut result, s, i);
    }

    result
}

/// Collapse multiple consecutive spaces into a single space.
fn collapse_multiple_spaces(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut prev_was_space = false;

    for c in s.chars() {
        if c == ' ' {
            if !prev_was_space {
                result.push(' ');
                prev_was_space = true;
            }
        } else {
            result.push(c);
            prev_was_space = false;
        }
    }

    result
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
        let result = strip_scoping_selectors(" ::ng-deep @top-left { content: \"Hamlet\";}", "h");
        assert!(result.starts_with(" "), "Expected leading space, got: {:?}", result);
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

    #[test]
    fn test_polyfill_next_selector_single_quotes() {
        let result =
            shim_css_text("polyfill-next-selector {content: 'x > y'} z {}", "contenta", "");
        assert!(result.contains("x[contenta]"), "Got: {}", result);
        assert!(result.contains("y[contenta]"), "Got: {}", result);
        assert!(!result.contains("polyfill-next-selector"), "Got: {}", result);
        assert!(!result.contains("z"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_next_selector_double_quotes() {
        let result =
            shim_css_text("polyfill-next-selector {content: \"x > y\"} z {}", "contenta", "");
        assert!(result.contains("x[contenta]"), "Got: {}", result);
        assert!(result.contains("y[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_next_selector_with_attribute() {
        let result = shim_css_text(
            "polyfill-next-selector {content: 'button[priority=\"1\"]'} z {}",
            "contenta",
            "",
        );
        assert!(result.contains("button[priority=\"1\"][contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_unscoped_rule_single_quotes() {
        let result = shim_css_text(
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
        let result = shim_css_text(
            "polyfill-unscoped-rule {content: \"#menu > .bar\";color: blue;}",
            "contenta",
            "",
        );
        assert!(result.contains("#menu > .bar"), "Got: {}", result);
        assert!(result.contains("color: blue"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_unscoped_rule_multiple() {
        let result = shim_css_text(
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
        let result = shim_css_text(
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
        let result = shim_css_text(
            "polyfill-rule {content: \":host.foo .bar\";color: blue;}",
            "contenta",
            "a-host",
        );
        assert!(result.contains(".foo[a-host]"), "Got: {}", result);
        assert!(result.contains(".bar[contenta]"), "Got: {}", result);
    }

    #[test]
    fn test_polyfill_rule_with_attribute() {
        let result = shim_css_text(
            "polyfill-rule {content: 'button[priority=\"1\"]'}",
            "contenta",
            "a-host",
        );
        assert!(result.contains("button[priority=\"1\"][contenta]"), "Got: {}", result);
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
}

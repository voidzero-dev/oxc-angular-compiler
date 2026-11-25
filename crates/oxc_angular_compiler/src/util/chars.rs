//! Character constants and utilities for lexing.
//!
//! Ported from Angular's `chars.ts`.

// Control characters
/// End of file marker
pub const EOF: char = '\0';
/// Horizontal tab
pub const TAB: char = '\t';
/// Line feed (newline)
pub const LF: char = '\n';
/// Space
pub const SPACE: char = ' ';

// Punctuation and operators
/// Exclamation mark `!`
pub const BANG: char = '!';
/// Double quote `"`
pub const DQ: char = '"';
/// Hash `#`
pub const HASH: char = '#';
/// Dollar sign `$`
pub const DOLLAR: char = '$';
/// Percent `%`
pub const PERCENT: char = '%';
/// Ampersand `&`
pub const AMPERSAND: char = '&';
/// Single quote `'`
pub const SQ: char = '\'';
/// Asterisk `*`
pub const STAR: char = '*';
/// Plus `+`
pub const PLUS: char = '+';
/// Minus `-`
pub const MINUS: char = '-';
/// Period `.`
pub const PERIOD: char = '.';
/// Forward slash `/`
pub const SLASH: char = '/';
/// Less than `<`
pub const LT: char = '<';
/// Equals `=`
pub const EQ: char = '=';
/// Greater than `>`
pub const GT: char = '>';
/// Question mark `?`
pub const QUESTION: char = '?';

// Brackets
/// Backslash `\`
pub const BACKSLASH: char = '\\';
/// Caret `^`
pub const CARET: char = '^';
/// Underscore `_`
pub const UNDERSCORE: char = '_';
/// Backtick `` ` ``
pub const BT: char = '`';
/// Left brace `{`
pub const LBRACE: char = '{';
/// Pipe/Bar `|`
pub const BAR: char = '|';
/// Right brace `}`
pub const RBRACE: char = '}';

/// Non-breaking space
pub const NBSP: char = '\u{00A0}';

/// Checks if a character is whitespace (tab through space, or NBSP).
#[inline]
pub fn is_whitespace(ch: char) -> bool {
    matches!(ch, TAB..=SPACE | NBSP)
}

/// Matches JavaScript's `\s`, which is also what `String.prototype.trim`,
/// `trimStart` and `trimEnd` strip. Unlike `char::is_whitespace`, it includes
/// U+FEFF and leaves out U+0085.
pub fn is_js_whitespace(c: char) -> bool {
    matches!(
        c,
        '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r' | ' ' | NBSP | '\u{1680}' | '\u{2000}'
            ..='\u{200A}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{202F}'
                | '\u{205F}'
                | '\u{3000}'
                | '\u{FEFF}'
    )
}

/// Checks if a character is a decimal digit (0-9).
#[inline]
pub fn is_digit(ch: char) -> bool {
    ch.is_ascii_digit()
}

/// Checks if a character is a hexadecimal digit (0-9, a-f, A-F).
#[inline]
pub fn is_ascii_hex_digit(ch: char) -> bool {
    ch.is_ascii_hexdigit()
}

/// Checks if a character is an octal digit (0-7).
#[inline]
pub fn is_octal_digit(ch: char) -> bool {
    matches!(ch, '0'..='7')
}

/// Checks if a character can start an identifier.
#[inline]
pub fn is_identifier_start(ch: char) -> bool {
    ch.is_ascii_alphabetic() || ch == UNDERSCORE || ch == DOLLAR
}

/// Checks if a character can be part of an identifier.
#[inline]
pub fn is_identifier_part(ch: char) -> bool {
    ch.is_ascii_alphanumeric() || ch == UNDERSCORE || ch == DOLLAR
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_whitespace() {
        assert!(is_whitespace(' '));
        assert!(is_whitespace('\t'));
        assert!(is_whitespace('\n'));
        assert!(is_whitespace(NBSP));
        assert!(!is_whitespace('a'));
    }

    #[test]
    fn test_is_digit() {
        assert!(is_digit('0'));
        assert!(is_digit('9'));
        assert!(!is_digit('a'));
    }

    #[test]
    fn test_is_identifier_start() {
        assert!(is_identifier_start('a'));
        assert!(is_identifier_start('Z'));
        assert!(is_identifier_start('_'));
        assert!(is_identifier_start('$'));
        assert!(!is_identifier_start('0'));
    }

    #[test]
    fn test_is_identifier_part() {
        assert!(is_identifier_part('a'));
        assert!(is_identifier_part('0'));
        assert!(is_identifier_part('_'));
        assert!(is_identifier_part('$'));
        assert!(!is_identifier_part('-'));
    }
}

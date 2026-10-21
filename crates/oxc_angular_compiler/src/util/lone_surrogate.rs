//! Strings with lone surrogates (`'\uD800'`).
//!
//! A JavaScript string can hold a lone surrogate code unit, which a Rust
//! `&str` can't. ngtsc reads such a string like any other and prints it as a
//! `\uXXXX` escape. To carry one through the compiler's `&str` names, each lone
//! code unit `0xD800 + k` is stored as two characters: the noncharacter
//! U+FDD0, which Unicode reserves for a program's internal use, then the
//! private-use character `U+10F800 + k`. The string printers turn that pair
//! back into `\uXXXX`; either character on its own is printed as itself.
//!
//! A source string that really holds U+FDD0 followed by a character from
//! U+10F800 to U+10FFFF reads as a stored lone surrogate too. Both are
//! reserved code points, so no real name is expected to have that pair.

use std::borrow::Cow;

use oxc_allocator::Allocator;
use oxc_str::{Ident, JSStr};

/// The first character of a stored lone surrogate.
const MARK: char = '\u{FDD0}';
/// The second character of a stored lone code unit `0xD800`.
const BASE: u32 = 0x10_F800;

/// `value` as a `&str`, with each lone surrogate stored as described above.
pub(crate) fn js_str_to_str(value: JSStr<'_>) -> Cow<'_, str> {
    if let Some(value) = value.as_str() {
        return Cow::Borrowed(value);
    }
    let mut out = String::with_capacity(value.len() + 4);
    for c in value.chars() {
        match c.to_char() {
            Some(c) => out.push(c),
            None => {
                out.push(MARK);
                out.push(char::from_u32(BASE + (c.to_u32() - 0xD800)).unwrap_or(MARK));
            }
        }
    }
    Cow::Owned(out)
}

/// [`js_str_to_str`] as an [`Ident`].
pub(crate) fn js_str_to_ident<'a>(value: JSStr<'a>, allocator: &'a Allocator) -> Ident<'a> {
    match js_str_to_str(value) {
        Cow::Borrowed(value) => Ident::from(value),
        Cow::Owned(value) => Ident::from(allocator.alloc_str(&value)),
    }
}

/// A character of a string, or a stored lone surrogate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Piece {
    Char(char),
    Lone(u16),
}

/// The pieces of `s`: its characters, with each stored lone surrogate as one
/// [`Piece::Lone`].
pub(crate) fn pieces(s: &str) -> impl Iterator<Item = Piece> + '_ {
    let mut chars = s.chars().peekable();
    std::iter::from_fn(move || {
        let c = chars.next()?;
        if c == MARK
            && let Some(unit) = chars.peek().and_then(|&next| stored_unit(next))
        {
            chars.next();
            return Some(Piece::Lone(unit));
        }
        Some(Piece::Char(c))
    })
}

/// Whether `s` stores a lone surrogate.
pub(crate) fn has_lone_surrogate(s: &str) -> bool {
    s.contains(MARK) && pieces(s).any(|piece| matches!(piece, Piece::Lone(_)))
}

/// The UTF-16 code units of `s`, with each stored lone surrogate as its unit.
pub(crate) fn encode_utf16(s: &str) -> impl Iterator<Item = u16> + '_ {
    pieces(s)
        .flat_map(|piece| match piece {
            Piece::Lone(unit) => [Some(unit), None],
            Piece::Char(c) => {
                let mut buf = [0; 2];
                let units = c.encode_utf16(&mut buf);
                [Some(units[0]), units.get(1).copied()]
            }
        })
        .flatten()
}

/// The lone code unit the second character `c` of a stored one stands for.
fn stored_unit(c: char) -> Option<u16> {
    // `BASE + 0x7FF` is `U+10FFFF`, the last code point.
    let offset = u16::try_from((c as u32).checked_sub(BASE)?).ok()?;
    Some(0xD800 + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_lone_surrogates() {
        let allocator = Allocator::default();
        let mut builder = oxc_str::JSStrBuilder::new_in(&&allocator);
        builder.push_str("x-");
        builder.push_code_unit(0xD800);
        builder.push('-');
        builder.push_code_unit(0xDFFF);
        let value = builder.into_js_str();
        let text = js_str_to_str(value);
        assert!(has_lone_surrogate(&text));
        assert_eq!(encode_utf16(&text).collect::<Vec<_>>(), [0x78, 0x2D, 0xD800, 0x2D, 0xDFFF]);
    }

    #[test]
    fn leaves_other_strings_alone() {
        assert!(matches!(js_str_to_str(JSStr::from("a😀\u{FFFD}")), Cow::Borrowed("a😀\u{FFFD}")));
        // Either character on its own, or in the other order, is itself.
        for text in
            ["\u{10F800}", "\u{FDD0}", "\u{FDD0}a", "\u{10F800}\u{FDD0}", "\u{FDD0}\u{10F7FF}"]
        {
            assert!(!has_lone_surrogate(text), "{text:?}");
            assert_eq!(
                encode_utf16(text).collect::<Vec<_>>(),
                text.encode_utf16().collect::<Vec<_>>()
            );
        }
    }
}

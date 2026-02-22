//! Deferred time parsing utilities.
//!
//! This module provides utilities for parsing time expressions from
//! deferred block parameters.
//!
//! Ported from Angular's `render3/r3_deferred_triggers.ts`.

/// Parses a time expression from a deferred trigger to milliseconds.
/// Returns None if it cannot be parsed.
///
/// Accepts formats:
/// - `500ms` - milliseconds
/// - `1s` - seconds
/// - `1.5s` - fractional seconds
/// - `100` - defaults to milliseconds
///
/// # Examples
/// ```
/// use oxc_angular_compiler::util::parse_deferred_time;
/// assert_eq!(parse_deferred_time("500ms"), Some(500));
/// assert_eq!(parse_deferred_time("1s"), Some(1000));
/// assert_eq!(parse_deferred_time("1.5s"), Some(1500));
/// assert_eq!(parse_deferred_time("100"), Some(100)); // default to ms
/// assert_eq!(parse_deferred_time("invalid"), None);
/// ```
pub fn parse_deferred_time(value: &str) -> Option<u32> {
    let value = value.trim();

    if value.is_empty() {
        return None;
    }

    // Extract the numeric part and optional unit
    let (num_str, multiplier) = if let Some(stripped) = value.strip_suffix("ms") {
        (stripped, 1.0)
    } else if let Some(stripped) = value.strip_suffix('s') {
        (stripped, 1000.0)
    } else {
        (value, 1.0) // default to milliseconds
    };

    // Validate and parse the numeric part
    if num_str.is_empty() {
        return None;
    }

    // Check that all characters are valid for a number
    let mut has_dot = false;
    for c in num_str.chars() {
        if c == '.' {
            if has_dot {
                return None; // multiple dots
            }
            has_dot = true;
        } else if !c.is_ascii_digit() {
            return None;
        }
    }

    let num: f64 = num_str.parse().ok()?;
    #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    Some((num * multiplier) as u32)
}

/// Gets the start position of trigger parameters (after the keyword).
/// This strips the keyword (like "minimum" or "after") and returns the rest.
pub fn get_trigger_parameters_start(expression: &str) -> usize {
    // Find the first whitespace after the keyword
    for (i, c) in expression.char_indices() {
        if c.is_whitespace() {
            // Skip all whitespace
            for (j, c2) in expression[i..].char_indices() {
                if !c2.is_whitespace() {
                    return i + j;
                }
            }
            return expression.len();
        }
    }
    expression.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_deferred_time_milliseconds() {
        assert_eq!(parse_deferred_time("500ms"), Some(500));
        assert_eq!(parse_deferred_time("100ms"), Some(100));
        assert_eq!(parse_deferred_time("0ms"), Some(0));
    }

    #[test]
    fn test_parse_deferred_time_seconds() {
        assert_eq!(parse_deferred_time("1s"), Some(1000));
        assert_eq!(parse_deferred_time("2s"), Some(2000));
        assert_eq!(parse_deferred_time("1.5s"), Some(1500));
        assert_eq!(parse_deferred_time("0.5s"), Some(500));
    }

    #[test]
    fn test_parse_deferred_time_no_unit() {
        assert_eq!(parse_deferred_time("100"), Some(100));
        assert_eq!(parse_deferred_time("500"), Some(500));
    }

    #[test]
    fn test_parse_deferred_time_invalid() {
        assert_eq!(parse_deferred_time("invalid"), None);
        assert_eq!(parse_deferred_time("abc123"), None);
        assert_eq!(parse_deferred_time(""), None);
    }

    #[test]
    fn test_parse_deferred_time_with_whitespace() {
        assert_eq!(parse_deferred_time("  500ms  "), Some(500));
        assert_eq!(parse_deferred_time("  1s  "), Some(1000));
    }
}

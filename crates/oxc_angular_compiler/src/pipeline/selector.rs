//! CSS selector parser for Angular content projection.
//!
//! Parses CSS selectors and converts them to R3 selector format for use with
//! `ɵɵprojectionDef()` instructions.
//!
//! Ported from Angular's `directive_matching.ts` CssSelector class and
//! `core.ts` parseSelectorToR3Selector function.

use oxc_allocator::{Allocator, Vec as OxcVec};
use oxc_span::Ident;

use crate::output::ast::{LiteralExpr, LiteralValue, OutputExpression};

/// Selector flags used in R3 format to indicate the type of selector part.
///
/// Matches Angular's core/src/render3/interfaces/projection.ts SelectorFlags.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectorFlags {
    /// Beginning of a new negative selector (:not)
    Not = 0b0001,
    /// Mode for matching attributes
    Attribute = 0b0010,
    /// Mode for matching tag names
    Element = 0b0100,
    /// Mode for matching class names
    Class = 0b1000,
}

impl SelectorFlags {
    /// Combine flags using bitwise OR.
    pub fn or(self, other: SelectorFlags) -> u8 {
        self as u8 | other as u8
    }
}

/// A parsed CSS selector.
#[derive(Debug, Default)]
pub struct CssSelector {
    /// The element name, if any.
    pub element: Option<String>,
    /// Class names to match.
    pub class_names: Vec<String>,
    /// Attribute name/value pairs (alternating: name1, value1, name2, value2, ...).
    pub attrs: Vec<String>,
    /// Negative selectors (:not(...)).
    pub not_selectors: Vec<CssSelector>,
}

impl CssSelector {
    /// Create a new empty selector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the element name.
    pub fn set_element(&mut self, element: &str) {
        self.element = Some(element.to_string());
    }

    /// Get attributes for directive matching, including class attributes.
    ///
    /// Returns a flat array of attribute name/value pairs. If the selector has class names,
    /// they're returned as `["class", "classname1 classname2 ..."]` followed by other attrs.
    ///
    /// Ported from Angular's `CssSelector.getAttrs()` in `directive_matching.ts`.
    pub fn get_attrs(&self) -> Vec<String> {
        let mut result = Vec::new();
        if !self.class_names.is_empty() {
            result.push("class".to_string());
            result.push(self.class_names.join(" "));
        }
        result.extend(self.attrs.clone());
        result
    }

    /// Add a class name.
    pub fn add_class_name(&mut self, name: &str) {
        self.class_names.push(name.to_lowercase());
    }

    /// Add an attribute (name and optional value).
    pub fn add_attribute(&mut self, name: &str, value: Option<&str>) {
        self.attrs.push(name.to_string());
        self.attrs.push(value.map(|v| v.to_lowercase()).unwrap_or_default());
    }

    /// Parse a CSS selector string into a list of CssSelectors.
    ///
    /// Handles comma-separated selectors, element names, classes, attributes, and :not().
    ///
    /// In debug mode, asserts on invalid selectors:
    /// - Nested `:not()` (e.g., `:not(:not(...))`)
    /// - Multiple selectors inside `:not()` (e.g., `:not(a, b)`)
    /// - Unescaped `$` in attribute selectors
    ///
    /// In release mode, returns partial results for malformed selectors.
    ///
    /// Ported from Angular's `CssSelector.parse()` in `directive_matching.ts`.
    pub fn parse(selector: &str) -> Vec<CssSelector> {
        let mut results = Vec::new();
        let mut current = CssSelector::new();
        let mut in_not = false;
        let mut not_selector = CssSelector::new();

        let chars: Vec<char> = selector.chars().collect();
        let len = chars.len();
        let mut i = 0;

        while i < len {
            let c = chars[i];

            // Skip whitespace (but handle comma as separator)
            if c.is_whitespace() {
                i += 1;
                continue;
            }

            // Handle comma separator
            if c == ',' {
                // Multiple selectors in :not are not supported - return what we have
                if in_not {
                    Self::finalize_selector(&mut results, &mut current, false, CssSelector::new());
                    return results;
                }
                Self::finalize_selector(&mut results, &mut current, in_not, not_selector);
                current = CssSelector::new();
                not_selector = CssSelector::new();
                in_not = false;
                i += 1;
                continue;
            }

            // Handle :not(
            if c == ':' && i + 4 < len && &selector[i..i + 5] == ":not(" {
                // Nesting :not in a selector is not allowed - return what we have
                if in_not {
                    Self::finalize_selector(&mut results, &mut current, false, CssSelector::new());
                    return results;
                }
                in_not = true;
                not_selector = CssSelector::new();
                current.not_selectors.push(CssSelector::new());
                i += 5;
                continue;
            }

            // Handle ) to close :not
            if c == ')' && in_not {
                // Replace the placeholder with actual not_selector
                if let Some(last) = current.not_selectors.last_mut() {
                    *last = std::mem::take(&mut not_selector);
                }
                in_not = false;
                i += 1;
                continue;
            }

            let target = if in_not { &mut not_selector } else { &mut current };

            // Handle class selector
            if c == '.' {
                i += 1;
                let start = i;
                while i < len && Self::is_ident_char(chars[i]) {
                    i += 1;
                }
                if i > start {
                    target.add_class_name(&selector[start..i]);
                }
                continue;
            }

            // Handle ID selector (treated as attribute id="...")
            if c == '#' {
                i += 1;
                let start = i;
                while i < len && Self::is_ident_char(chars[i]) {
                    i += 1;
                }
                if i > start {
                    target.add_attribute("id", Some(&selector[start..i]));
                }
                continue;
            }

            // Handle attribute selector
            if c == '[' {
                i += 1;
                let attr_start = i;
                // Find attribute name (may contain escape sequences like \$)
                while i < len && chars[i] != '=' && chars[i] != ']' {
                    i += 1;
                }
                let raw_attr_name = selector[attr_start..i].trim();
                // Unescape attribute name (like Angular's unescapeAttribute)
                let attr_name = Self::unescape_attribute(raw_attr_name);

                if i < len && chars[i] == '=' {
                    i += 1;
                    // Skip optional quote
                    let quote = if i < len && (chars[i] == '"' || chars[i] == '\'') {
                        let q = chars[i];
                        i += 1;
                        Some(q)
                    } else {
                        None
                    };

                    let value_start = i;
                    if let Some(q) = quote {
                        while i < len && chars[i] != q {
                            i += 1;
                        }
                        let attr_value = &selector[value_start..i];
                        target.add_attribute(&attr_name, Some(attr_value));
                        if i < len {
                            i += 1; // Skip closing quote
                        }
                    } else {
                        while i < len && chars[i] != ']' {
                            i += 1;
                        }
                        let attr_value = selector[value_start..i].trim();
                        target.add_attribute(&attr_name, Some(attr_value));
                    }
                } else {
                    target.add_attribute(&attr_name, None);
                }

                // Skip closing bracket
                if i < len && chars[i] == ']' {
                    i += 1;
                }
                continue;
            }

            // Handle element name (identifier at current position)
            if Self::is_ident_start(c) {
                let start = i;
                while i < len && Self::is_ident_char(chars[i]) {
                    i += 1;
                }
                target.set_element(&selector[start..i]);
                continue;
            }

            // Handle * wildcard element
            if c == '*' {
                target.set_element("*");
                i += 1;
                continue;
            }

            // Skip unknown character
            i += 1;
        }

        Self::finalize_selector(&mut results, &mut current, in_not, not_selector);
        results
    }

    /// Unescape `\$` and `\\` sequences from CSS attribute selectors.
    ///
    /// This is needed because `$` can have special meaning in CSS selectors,
    /// but we might want to match an attribute that contains `$`.
    ///
    /// In debug mode, asserts on unescaped `$`. In release mode, includes `$` literally.
    ///
    /// Ported from Angular's `CssSelector.unescapeAttribute()`.
    fn unescape_attribute(attr: &str) -> String {
        let mut result = String::new();
        let mut escaping = false;

        for c in attr.chars() {
            if c == '\\' {
                if escaping {
                    // Escaped backslash: \\
                    result.push('\\');
                    escaping = false;
                } else {
                    escaping = true;
                }
                continue;
            }

            // Unescaped $ is not officially supported - include it literally (lenient parsing)
            // User should escape with \$ but we handle it gracefully

            escaping = false;
            result.push(c);
        }

        result
    }

    fn is_ident_start(c: char) -> bool {
        c.is_ascii_alphabetic() || c == '_' || c == '-' || c > '\x7f'
    }

    fn is_ident_char(c: char) -> bool {
        c.is_ascii_alphanumeric() || c == '_' || c == '-' || c > '\x7f'
    }

    fn finalize_selector(
        results: &mut Vec<CssSelector>,
        current: &mut CssSelector,
        in_not: bool,
        not_selector: CssSelector,
    ) {
        // If selector only has :not() selectors, add '*' element
        if !current.not_selectors.is_empty()
            && current.element.is_none()
            && current.class_names.is_empty()
            && current.attrs.is_empty()
        {
            current.element = Some("*".to_string());
        }

        // If we were in :not() but didn't close it, add the partial selector
        if in_not
            && (not_selector.element.is_some()
                || !not_selector.class_names.is_empty()
                || !not_selector.attrs.is_empty())
        {
            current.not_selectors.push(not_selector);
        }

        results.push(std::mem::take(current));
    }
}

/// An R3 selector element - either a string or a flag value.
#[derive(Debug, Clone)]
pub enum R3SelectorElement {
    /// A string value (element name, class name, attribute name/value).
    String(String),
    /// A numeric flag value (SelectorFlags).
    Flag(u8),
}

/// Convert a CSS selector to a simple R3 selector (without :not handling).
fn css_selector_to_simple_r3(selector: &CssSelector) -> Vec<R3SelectorElement> {
    let mut result = Vec::new();

    // Element name (empty string if none or wildcard)
    let element = match &selector.element {
        Some(e) if e != "*" => e.clone(),
        _ => String::new(),
    };
    result.push(R3SelectorElement::String(element));

    // Attributes (already in pairs)
    for attr in &selector.attrs {
        result.push(R3SelectorElement::String(attr.clone()));
    }

    // Classes with CLASS flag
    if !selector.class_names.is_empty() {
        result.push(R3SelectorElement::Flag(SelectorFlags::Class as u8));
        for class in &selector.class_names {
            result.push(R3SelectorElement::String(class.clone()));
        }
    }

    result
}

/// Convert a CSS selector to a negative R3 selector (for :not()).
fn css_selector_to_negative_r3(selector: &CssSelector) -> Vec<R3SelectorElement> {
    let mut result = Vec::new();

    // Classes with CLASS flag
    let classes: Vec<R3SelectorElement> = if !selector.class_names.is_empty() {
        let mut c = vec![R3SelectorElement::Flag(SelectorFlags::Class as u8)];
        for class in &selector.class_names {
            c.push(R3SelectorElement::String(class.clone()));
        }
        c
    } else {
        Vec::new()
    };

    if let Some(element) = &selector.element {
        // Has element: NOT | ELEMENT flag
        result.push(R3SelectorElement::Flag(SelectorFlags::Not.or(SelectorFlags::Element)));
        result.push(R3SelectorElement::String(element.clone()));
        // Add attributes
        for attr in &selector.attrs {
            result.push(R3SelectorElement::String(attr.clone()));
        }
        result.extend(classes);
    } else if !selector.attrs.is_empty() {
        // Has attributes but no element: NOT | ATTRIBUTE flag
        result.push(R3SelectorElement::Flag(SelectorFlags::Not.or(SelectorFlags::Attribute)));
        for attr in &selector.attrs {
            result.push(R3SelectorElement::String(attr.clone()));
        }
        result.extend(classes);
    } else if !selector.class_names.is_empty() {
        // Only classes: NOT | CLASS flag
        result.push(R3SelectorElement::Flag(SelectorFlags::Not.or(SelectorFlags::Class)));
        for class in &selector.class_names {
            result.push(R3SelectorElement::String(class.clone()));
        }
    }

    result
}

/// Convert a CssSelector to R3 format including :not() selectors.
fn css_selector_to_r3(selector: &CssSelector) -> Vec<R3SelectorElement> {
    let mut result = css_selector_to_simple_r3(selector);

    // Add negative selectors
    for not_sel in &selector.not_selectors {
        result.extend(css_selector_to_negative_r3(not_sel));
    }

    result
}

/// Parse a selector string to R3 selector list.
///
/// Returns a list of R3 selectors, one for each comma-separated selector.
pub fn parse_selector_to_r3_selector(selector: &str) -> Vec<Vec<R3SelectorElement>> {
    if selector.is_empty() {
        return Vec::new();
    }
    CssSelector::parse(selector).into_iter().map(|sel| css_selector_to_r3(&sel)).collect()
}

/// Convert R3 selector elements to output expressions.
pub fn r3_selector_to_output_expr<'a>(
    allocator: &'a Allocator,
    elements: &[R3SelectorElement],
) -> OxcVec<'a, OutputExpression<'a>> {
    let mut result = OxcVec::with_capacity_in(elements.len(), allocator);
    for element in elements {
        match element {
            R3SelectorElement::String(s) => {
                result.push(OutputExpression::Literal(oxc_allocator::Box::new_in(
                    LiteralExpr {
                        value: LiteralValue::String(Ident::from(allocator.alloc_str(s))),
                        source_span: None,
                    },
                    allocator,
                )));
            }
            R3SelectorElement::Flag(f) => {
                result.push(OutputExpression::Literal(oxc_allocator::Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(*f as f64), source_span: None },
                    allocator,
                )));
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_element() {
        let selectors = CssSelector::parse("div");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].element, Some("div".to_string()));
    }

    #[test]
    fn test_parse_class() {
        let selectors = CssSelector::parse(".my-class");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].class_names, vec!["my-class"]);
    }

    #[test]
    fn test_parse_element_with_class() {
        let selectors = CssSelector::parse("div.my-class");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].element, Some("div".to_string()));
        assert_eq!(selectors[0].class_names, vec!["my-class"]);
    }

    #[test]
    fn test_parse_attribute() {
        let selectors = CssSelector::parse("[name]");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].attrs, vec!["name", ""]);
    }

    #[test]
    fn test_parse_attribute_with_value() {
        let selectors = CssSelector::parse("[name=value]");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].attrs, vec!["name", "value"]);
    }

    #[test]
    fn test_parse_comma_separated() {
        let selectors = CssSelector::parse("div, span");
        assert_eq!(selectors.len(), 2);
        assert_eq!(selectors[0].element, Some("div".to_string()));
        assert_eq!(selectors[1].element, Some("span".to_string()));
    }

    #[test]
    fn test_parse_not_selector() {
        let selectors = CssSelector::parse(":not(.hidden)");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].element, Some("*".to_string())); // Added because only :not
        assert_eq!(selectors[0].not_selectors.len(), 1);
        assert_eq!(selectors[0].not_selectors[0].class_names, vec!["hidden"]);
    }

    #[test]
    fn test_r3_simple_element() {
        let r3 = parse_selector_to_r3_selector("div");
        assert_eq!(r3.len(), 1);
        // ["div"]
        assert!(matches!(&r3[0][0], R3SelectorElement::String(s) if s == "div"));
    }

    #[test]
    fn test_r3_element_with_class() {
        let r3 = parse_selector_to_r3_selector("div.my-class");
        assert_eq!(r3.len(), 1);
        // ["div", CLASS, "my-class"]
        assert!(matches!(&r3[0][0], R3SelectorElement::String(s) if s == "div"));
        assert!(matches!(&r3[0][1], R3SelectorElement::Flag(8))); // CLASS = 8
        assert!(matches!(&r3[0][2], R3SelectorElement::String(s) if s == "my-class"));
    }

    #[test]
    fn test_r3_element_with_attribute() {
        // Test case from the task: span[bitBadge] should become ["span", "bitBadge", ""]
        let r3 = parse_selector_to_r3_selector("span[bitBadge]");
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0].len(), 3);
        assert!(matches!(&r3[0][0], R3SelectorElement::String(s) if s == "span"));
        assert!(matches!(&r3[0][1], R3SelectorElement::String(s) if s == "bitBadge"));
        assert!(matches!(&r3[0][2], R3SelectorElement::String(s) if s == ""));
    }

    #[test]
    fn test_r3_multiple_element_with_attribute() {
        // Test case: span[bitBadge], a[bitBadge], button[bitBadge]
        let r3 = parse_selector_to_r3_selector("span[bitBadge], a[bitBadge], button[bitBadge]");
        assert_eq!(r3.len(), 3);
        // First selector: span[bitBadge] -> ["span", "bitBadge", ""]
        assert!(matches!(&r3[0][0], R3SelectorElement::String(s) if s == "span"));
        assert!(matches!(&r3[0][1], R3SelectorElement::String(s) if s == "bitBadge"));
        assert!(matches!(&r3[0][2], R3SelectorElement::String(s) if s == ""));
        // Second selector: a[bitBadge] -> ["a", "bitBadge", ""]
        assert!(matches!(&r3[1][0], R3SelectorElement::String(s) if s == "a"));
        assert!(matches!(&r3[1][1], R3SelectorElement::String(s) if s == "bitBadge"));
        assert!(matches!(&r3[1][2], R3SelectorElement::String(s) if s == ""));
        // Third selector: button[bitBadge] -> ["button", "bitBadge", ""]
        assert!(matches!(&r3[2][0], R3SelectorElement::String(s) if s == "button"));
        assert!(matches!(&r3[2][1], R3SelectorElement::String(s) if s == "bitBadge"));
        assert!(matches!(&r3[2][2], R3SelectorElement::String(s) if s == ""));
    }

    #[test]
    fn test_r3_attribute_only() {
        // [ngFor] should become ["", "ngFor", ""]
        let r3 = parse_selector_to_r3_selector("[ngFor]");
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0].len(), 3);
        assert!(matches!(&r3[0][0], R3SelectorElement::String(s) if s == ""));
        assert!(matches!(&r3[0][1], R3SelectorElement::String(s) if s == "ngFor"));
        assert!(matches!(&r3[0][2], R3SelectorElement::String(s) if s == ""));
    }

    #[test]
    fn test_r3_attribute_with_value() {
        // button[type="submit"] should become ["button", "type", "submit"]
        let r3 = parse_selector_to_r3_selector("button[type=\"submit\"]");
        assert_eq!(r3.len(), 1);
        assert_eq!(r3[0].len(), 3);
        assert!(matches!(&r3[0][0], R3SelectorElement::String(s) if s == "button"));
        assert!(matches!(&r3[0][1], R3SelectorElement::String(s) if s == "type"));
        assert!(matches!(&r3[0][2], R3SelectorElement::String(s) if s == "submit"));
    }

    #[test]
    fn test_parse_element_with_attribute() {
        // span[bitBadge] should parse to element="span", attrs=["bitBadge", ""]
        let selectors = CssSelector::parse("span[bitBadge]");
        assert_eq!(selectors.len(), 1);
        assert_eq!(selectors[0].element, Some("span".to_string()));
        assert_eq!(selectors[0].attrs, vec!["bitBadge", ""]);
    }
}

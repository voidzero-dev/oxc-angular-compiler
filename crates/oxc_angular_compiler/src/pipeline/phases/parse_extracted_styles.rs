//! Parse extracted styles phase.
//!
//! Parses extracted style and class attributes into separate ExtractedAttributeOps
//! per style or class property.
//!
//! Ported from Angular's `template/pipeline/src/phases/parse_extracted_styles.ts`.

use oxc_allocator::Box;
use oxc_str::Ident;
use rustc_hash::FxHashSet;

use crate::ast::expression::{
    AbsoluteSourceSpan, AngularExpression, LiteralPrimitive, LiteralValue, ParseSpan,
};
use crate::ast::r3::SecurityContext;
use crate::ir::enums::{BindingKind, TemplateKind};
use crate::ir::expression::IrExpression;
use crate::ir::ops::{CreateOp, CreateOpBase, ExtractedAttributeOp, XrefId};
use crate::output::ast::OutputExpression;
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

/// Parses extracted style and class attribute strings.
///
/// This phase:
/// - Parses `style="color: red; height: auto"` into individual style property ops
/// - Parses `class="foo bar baz"` into individual class name ops
pub fn parse_extracted_styles(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect all view xrefs
    let view_xrefs: Vec<XrefId> = job.all_views().map(|v| v.xref).collect();

    // First pass: collect element/container xrefs that are structural templates or conditionals
    // (we skip style/class parsing for these - they need the raw attributes for content projection)
    let mut structural_templates: FxHashSet<XrefId> = FxHashSet::default();

    for view_xref in &view_xrefs {
        if let Some(view) = job.view(*view_xref) {
            for op in view.create.iter() {
                match op {
                    CreateOp::Template(template) => {
                        if template.template_kind == TemplateKind::Structural {
                            structural_templates.insert(template.xref);
                        }
                    }
                    // Conditional and ConditionalBranch ops only skip style/class parsing
                    // if they are structural templates (not Block templates from @if/@switch)
                    CreateOp::Conditional(cond) => {
                        if cond.template_kind == TemplateKind::Structural {
                            structural_templates.insert(cond.xref);
                        }
                    }
                    CreateOp::ConditionalBranch(branch) => {
                        if branch.template_kind == TemplateKind::Structural {
                            structural_templates.insert(branch.xref);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    // Second pass: collect style/class attributes to parse
    // We collect first because we need to insert new ops and remove old ones
    for view_xref in &view_xrefs {
        // Collect ops to process
        let ops_to_process: std::vec::Vec<(XrefId, Ident<'_>, String)> = {
            let Some(view) = job.view(*view_xref) else {
                continue;
            };

            view.create
                .iter()
                .filter_map(|op| {
                    if let CreateOp::ExtractedAttribute(attr) = op {
                        if attr.binding_kind == BindingKind::Attribute {
                            // Check if value is a string literal
                            if let Some(ref value) = attr.value {
                                let string_value = extract_string_value(value.as_ref());
                                if let Some(s) = string_value {
                                    let name = attr.name.as_str();
                                    if name == "style" || name == "class" {
                                        // Skip structural templates
                                        if structural_templates.contains(&attr.target) {
                                            return None;
                                        }
                                        return Some((attr.target, attr.name.clone(), s));
                                    }
                                }
                            }
                        }
                    }
                    None
                })
                .collect()
        };

        // Process collected ops
        if let Some(view) = job.view_mut(*view_xref) {
            // We need to handle this carefully due to the doubly-linked list
            // For now, just insert new ops and mark old ones for removal

            for (target, name, value) in ops_to_process {
                if name.as_str() == "style" {
                    let parsed_styles = parse_style_string(&value);

                    // Insert new style property ops
                    for (prop_name, prop_value) in parsed_styles {
                        let value_expr = Box::new_in(
                            IrExpression::Ast(Box::new_in(
                                AngularExpression::LiteralPrimitive(Box::new_in(
                                    LiteralPrimitive {
                                        span: ParseSpan::new(0, 0),
                                        source_span: AbsoluteSourceSpan::new(0, 0),
                                        value: LiteralValue::String(Ident::from(
                                            allocator.alloc_str(&prop_value),
                                        )),
                                    },
                                    &allocator,
                                )),
                                &allocator,
                            )),
                            &allocator,
                        );

                        let new_op = CreateOp::ExtractedAttribute(ExtractedAttributeOp {
                            base: CreateOpBase { prev: None, next: None, source_span: None },
                            target,
                            binding_kind: BindingKind::StyleProperty,
                            namespace: None,
                            name: Ident::from(allocator.alloc_str(&prop_name)),
                            value: Some(value_expr),
                            security_context: SecurityContext::Style,
                            truthy_expression: false,
                            i18n_message: None,
                            i18n_context: None,
                            trusted_value_fn: None,
                        });

                        view.create.push(new_op);
                    }
                } else if name.as_str() == "class" {
                    // Match JavaScript's "".split(/\s+/g) behavior which returns [""]
                    // for empty strings, while Rust's split_whitespace() returns empty iterator.
                    // This is important for directive matching - class="" should still create
                    // an entry in the consts array.
                    let trimmed = value.trim();
                    let parsed_classes: std::vec::Vec<_> = if trimmed.is_empty() {
                        vec![String::new()] // Match JS behavior: [""]
                    } else {
                        trimmed.split_whitespace().map(String::from).collect()
                    };

                    // Insert new class name ops
                    for class_name in parsed_classes {
                        let new_op = CreateOp::ExtractedAttribute(ExtractedAttributeOp {
                            base: CreateOpBase { prev: None, next: None, source_span: None },
                            target,
                            binding_kind: BindingKind::ClassName,
                            namespace: None,
                            name: Ident::from(allocator.alloc_str(&class_name)),
                            value: None,
                            security_context: SecurityContext::None,
                            truthy_expression: false,
                            i18n_message: None,
                            i18n_context: None,
                            trusted_value_fn: None,
                        });

                        view.create.push(new_op);
                    }
                }
            }

            // Remove original style/class ExtractedAttribute ops
            let mut cursor = view.create.cursor_front();
            loop {
                let should_remove = if let Some(op) = cursor.current() {
                    if let CreateOp::ExtractedAttribute(attr) = op {
                        attr.binding_kind == BindingKind::Attribute
                            && (attr.name.as_str() == "style" || attr.name.as_str() == "class")
                            && !structural_templates.contains(&attr.target)
                    } else {
                        false
                    }
                } else {
                    break;
                };

                if should_remove {
                    cursor.remove_current();
                } else if !cursor.move_next() {
                    break;
                }
            }
        }
    }
}

/// Parses a CSS style string into property-value pairs.
///
/// A 1:1 port of Angular's `parse()` (`parse_extracted_styles.ts`, shared with
/// the Domino fork's `style_parser.js`), quirks included:
/// - `;` and `:` inside parentheses or quotes are part of the value.
/// - A declaration with an empty value is kept (`a:;` gives `a` = `""`).
/// - A `;` before any `name:` does not start a new property, so `x;a:b` gives
///   the property `x;a`, and `a:b;;c:d` gives `;c` for the second one.
/// - The escaped-quote check reads the quote itself rather than the character
///   before it, so a matching quote always closes, even after a backslash.
pub fn parse_style_string(value: &str) -> std::vec::Vec<(String, String)> {
    let mut styles = std::vec::Vec::new();
    // Every character the parser looks at is ASCII, so byte offsets are char
    // boundaries and match Angular's UTF-16 offsets for slicing purposes.
    let bytes = value.as_bytes();

    let mut i = 0;
    let mut paren_depth = 0i32;
    // `0` means not inside a quote.
    let mut quote = 0u8;
    let mut value_start = 0;
    let mut prop_start = 0;
    // Angular's `currentProp` is `string | null`; both `null` and `''` are
    // falsy, so an empty string stands for both.
    let mut current_prop = String::new();
    while i < bytes.len() {
        let token = bytes[i];
        i += 1;
        match token {
            b'(' => paren_depth += 1,
            b')' => paren_depth -= 1,
            b'\'' | b'"' => {
                if quote == 0 {
                    quote = token;
                } else if quote == token {
                    quote = 0;
                }
            }
            b':' if current_prop.is_empty() && paren_depth == 0 && quote == 0 => {
                current_prop = hyphenate(js_trim(&value[prop_start..i - 1]));
                value_start = i;
            }
            b';' if !current_prop.is_empty()
                && value_start > 0
                && paren_depth == 0
                && quote == 0 =>
            {
                let style_val = js_trim(&value[value_start..i - 1]).to_string();
                styles.push((std::mem::take(&mut current_prop), style_val));
                prop_start = i;
                value_start = 0;
            }
            _ => {}
        }
    }

    if !current_prop.is_empty() && value_start > 0 {
        let style_val = js_trim(&value[value_start..]).to_string();
        styles.push((current_prop, style_val));
    }

    styles
}

/// `String.prototype.trim`: Unicode white space and line terminators, plus
/// U+FEFF but not U+0085.
fn js_trim(value: &str) -> &str {
    value.trim_matches(|c: char| (c.is_whitespace() && c != '\u{85}') || c == '\u{FEFF}')
}

/// Converts camelCase to kebab-case.
///
/// Matches Angular's `hyphenate` function from `parse_extracted_styles.ts`.
/// Inserts a hyphen between an ASCII lowercase letter and a following ASCII
/// uppercase letter (`/[a-z][A-Z]/g`), then lowercases the whole string like
/// `String.prototype.toLowerCase`.
///
/// Example: `backgroundColor` -> `background-color`
pub fn hyphenate(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 4);
    let mut prev_lower = false;
    for ch in value.chars() {
        if ch.is_ascii_uppercase() && prev_lower {
            result.push('-');
        }
        prev_lower = ch.is_ascii_lowercase();
        result.push(ch);
    }
    result.to_lowercase()
}

/// Extracts a string value from an IrExpression.
///
/// Handles both:
/// - `IrExpression::Ast(AngularExpression::LiteralPrimitive(...))` - from R3 AST
/// - `IrExpression::OutputExpr(OutputExpression::Literal(...))` - from ingestion
fn extract_string_value(expr: &IrExpression<'_>) -> Option<String> {
    match expr {
        // Handle AST literals (from R3 parsing)
        IrExpression::Ast(ast_expr) => {
            if let AngularExpression::LiteralPrimitive(prim) = ast_expr.as_ref() {
                if let LiteralValue::String(s) = &prim.value {
                    return Some(s.to_string());
                }
            }
            None
        }
        // Handle Output literals (from ingest_static_attributes)
        IrExpression::OutputExpr(output_expr) => {
            if let OutputExpression::Literal(lit) = output_expr.as_ref() {
                if let crate::output::ast::LiteralValue::String(s) = &lit.value {
                    return Some(s.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

/// Parses extracted styles for host binding compilation.
///
/// Host version - processes style and class attributes in the root create list.
/// Matches Angular's parseExtractedStyles for host bindings (Kind.Both).
pub fn parse_extracted_styles_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    // Collect style/class attributes to parse
    let ops_to_process: std::vec::Vec<(XrefId, Ident<'_>, String)> = job
        .root
        .create
        .iter()
        .filter_map(|op| {
            if let CreateOp::ExtractedAttribute(attr) = op {
                if attr.binding_kind == BindingKind::Attribute {
                    if let Some(ref value) = attr.value {
                        let string_value = extract_string_value(value.as_ref());
                        if let Some(s) = string_value {
                            let name = attr.name.as_str();
                            if name == "style" || name == "class" {
                                return Some((attr.target, attr.name.clone(), s));
                            }
                        }
                    }
                }
            }
            None
        })
        .collect();

    // Process collected ops
    for (target, name, value) in ops_to_process {
        if name.as_str() == "style" {
            let parsed_styles = parse_style_string(&value);

            // Insert new style property ops
            for (prop_name, prop_value) in parsed_styles {
                let value_expr = Box::new_in(
                    IrExpression::Ast(Box::new_in(
                        AngularExpression::LiteralPrimitive(Box::new_in(
                            LiteralPrimitive {
                                span: ParseSpan::new(0, 0),
                                source_span: AbsoluteSourceSpan::new(0, 0),
                                value: LiteralValue::String(Ident::from(
                                    allocator.alloc_str(&prop_value),
                                )),
                            },
                            &allocator,
                        )),
                        &allocator,
                    )),
                    &allocator,
                );

                let new_op = CreateOp::ExtractedAttribute(ExtractedAttributeOp {
                    base: CreateOpBase { prev: None, next: None, source_span: None },
                    target,
                    binding_kind: BindingKind::StyleProperty,
                    namespace: None,
                    name: Ident::from(allocator.alloc_str(&prop_name)),
                    value: Some(value_expr),
                    security_context: SecurityContext::Style,
                    truthy_expression: false,
                    i18n_message: None,
                    i18n_context: None,
                    trusted_value_fn: None,
                });

                job.root.create.push(new_op);
            }
        } else if name.as_str() == "class" {
            // Match JavaScript's "".split(/\s+/g) behavior
            let trimmed = value.trim();
            let parsed_classes: std::vec::Vec<_> = if trimmed.is_empty() {
                vec![String::new()]
            } else {
                trimmed.split_whitespace().map(String::from).collect()
            };

            // Insert new class name ops
            for class_name in parsed_classes {
                let new_op = CreateOp::ExtractedAttribute(ExtractedAttributeOp {
                    base: CreateOpBase { prev: None, next: None, source_span: None },
                    target,
                    binding_kind: BindingKind::ClassName,
                    namespace: None,
                    name: Ident::from(allocator.alloc_str(&class_name)),
                    value: None,
                    security_context: SecurityContext::None,
                    truthy_expression: false,
                    i18n_message: None,
                    i18n_context: None,
                    trusted_value_fn: None,
                });

                job.root.create.push(new_op);
            }
        }
    }

    // Remove original style/class ExtractedAttribute ops
    let mut cursor = job.root.create.cursor_front();
    loop {
        let should_remove = if let Some(op) = cursor.current() {
            if let CreateOp::ExtractedAttribute(attr) = op {
                attr.binding_kind == BindingKind::Attribute
                    && (attr.name.as_str() == "style" || attr.name.as_str() == "class")
            } else {
                false
            }
        } else {
            break;
        };

        if should_remove {
            cursor.remove_current();
        } else if !cursor.move_next() {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{hyphenate, parse_style_string};

    /// The `[name, value, name, value, ...]` array Angular's `parse()` returns.
    fn parse(value: &str) -> std::vec::Vec<String> {
        parse_style_string(value).into_iter().flat_map(|(k, v)| [k, v]).collect()
    }

    // Expected values are what Angular 22.2's `parse()` returns for the same input.
    #[test]
    fn parses_like_angular_style_parser() {
        assert_eq!(parse("color: red; height: auto"), ["color", "red", "height", "auto"]);
        assert_eq!(parse("width:100px;"), ["width", "100px"]);
        assert!(parse("").is_empty());
        assert!(parse("color").is_empty());
        // Empty values are kept.
        assert_eq!(parse("a:;b:c"), ["a", "", "b", "c"]);
        assert_eq!(parse("a:"), ["a", ""]);
        // A `;` before `name:` does not start a new property.
        assert_eq!(parse("x;a:b"), ["x;a", "b"]);
        assert_eq!(parse("a:b;;c:d"), ["a", "b", ";c", "d"]);
        assert_eq!(parse(";a:b"), [";a", "b"]);
        // An empty name does not take the value; the next `:` starts again.
        assert_eq!(parse(":a:b"), [":a", "b"]);
        // `;` and `:` inside parentheses and quotes stay in the value.
        assert_eq!(
            parse("background: url(\"a;b:c\"); content: 'x;y'"),
            ["background", "url(\"a;b:c\")", "content", "'x;y'"]
        );
        // A backslash does not keep a matching quote from closing.
        assert_eq!(parse(r#"content: "a\"; color: red"#), ["content", r#""a\""#, "color", "red"]);
        // camelCase names are hyphenated.
        assert_eq!(parse("backgroundColor: red"), ["background-color", "red"]);
    }

    #[test]
    fn hyphenates_like_angular() {
        assert_eq!(hyphenate("backgroundColor"), "background-color");
        assert_eq!(hyphenate("WebkitAppearance"), "webkit-appearance");
        assert_eq!(hyphenate("aBC"), "a-bc");
        // `toLowerCase` lowercases non-ASCII letters too.
        assert_eq!(hyphenate("ÄB"), "äb");
    }
}

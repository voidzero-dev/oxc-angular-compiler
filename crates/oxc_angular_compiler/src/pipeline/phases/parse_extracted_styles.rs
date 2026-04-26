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
                                    allocator,
                                )),
                                allocator,
                            )),
                            allocator,
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
/// Handles:
/// - Basic syntax: "color: red; height: auto"
/// - Parentheses: "background: url(foo.png)"
/// - Quoted values: "content: 'hello'"
fn parse_style_string(value: &str) -> std::vec::Vec<(String, String)> {
    let mut styles = std::vec::Vec::new();
    let mut chars = value.chars().peekable();

    let mut paren_depth = 0;
    let mut quote_char: Option<char> = None;
    let mut current_prop = String::new();
    let mut current_value = String::new();
    let mut in_value = false;
    let mut prev_char = '\0';

    while let Some(ch) = chars.next() {
        match ch {
            '(' => {
                paren_depth += 1;
                if in_value {
                    current_value.push(ch);
                } else {
                    current_prop.push(ch);
                }
            }
            ')' => {
                paren_depth -= 1;
                if in_value {
                    current_value.push(ch);
                } else {
                    current_prop.push(ch);
                }
            }
            '\'' | '"' => {
                if quote_char.is_none() {
                    quote_char = Some(ch);
                } else if quote_char == Some(ch) && prev_char != '\\' {
                    quote_char = None;
                }
                if in_value {
                    current_value.push(ch);
                } else {
                    current_prop.push(ch);
                }
            }
            ':' => {
                if quote_char.is_none() && paren_depth == 0 && !in_value {
                    in_value = true;
                } else if in_value {
                    current_value.push(ch);
                } else {
                    current_prop.push(ch);
                }
            }
            ';' => {
                if quote_char.is_none() && paren_depth == 0 {
                    // End of property
                    let prop = hyphenate(current_prop.trim());
                    let val = current_value.trim().to_string();
                    if !prop.is_empty() && !val.is_empty() {
                        styles.push((prop, val));
                    }
                    current_prop.clear();
                    current_value.clear();
                    in_value = false;
                } else if in_value {
                    current_value.push(ch);
                } else {
                    current_prop.push(ch);
                }
            }
            _ => {
                if in_value {
                    current_value.push(ch);
                } else {
                    current_prop.push(ch);
                }
            }
        }
        prev_char = ch;
    }

    // Handle last property without trailing semicolon
    let prop = hyphenate(current_prop.trim());
    let val = current_value.trim().to_string();
    if !prop.is_empty() && !val.is_empty() {
        styles.push((prop, val));
    }

    styles
}

/// Converts camelCase to kebab-case.
///
/// Matches Angular's `hyphenate` function from `parse_extracted_styles.ts`.
/// Inserts a hyphen before uppercase letters that follow lowercase letters,
/// then converts the entire string to lowercase.
///
/// Example: `backgroundColor` -> `background-color`
pub fn hyphenate(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 4);
    let chars: Vec<char> = value.chars().collect();

    for (i, &ch) in chars.iter().enumerate() {
        // Insert hyphen if current char is uppercase and previous char is lowercase
        // This matches TypeScript's /[a-z][A-Z]/g regex behavior
        if ch.is_ascii_uppercase() && i > 0 && chars[i - 1].is_ascii_lowercase() {
            result.push('-');
        }
        result.push(ch.to_ascii_lowercase());
    }
    result
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
                            allocator,
                        )),
                        allocator,
                    )),
                    allocator,
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

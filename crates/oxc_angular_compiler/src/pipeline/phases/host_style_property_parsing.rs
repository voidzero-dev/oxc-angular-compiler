//! Host style property parsing phase.
//!
//! Parses host style bindings for host binding compilation. Host bindings are
//! compiled using a different parser entrypoint, so we need extra parsing for
//! host style properties.
//!
//! This phase handles:
//! - `style.propName` bindings → StyleProperty
//! - `class.className` bindings → ClassName
//! - `!important` suffix removal
//! - camelCase to kebab-case conversion for CSS properties
//! - Unit suffix parsing (e.g., `style.width.px`)
//!
//! Ported from Angular's `template/pipeline/src/phases/host_style_property_parsing.ts`.

use oxc_str::Ident;

use crate::ir::enums::BindingKind;
use crate::ir::ops::UpdateOp;
use crate::pipeline::compilation::{ComponentCompilationJob, HostBindingCompilationJob};

const STYLE_DOT: &str = "style.";
const CLASS_DOT: &str = "class.";
const STYLE_BANG: &str = "style!";
const CLASS_BANG: &str = "class!";
const BANG_IMPORTANT: &str = "!important";

/// Parses host style property bindings.
///
/// This phase transforms generic property bindings from host metadata
/// into specific style/class bindings with proper names and units.
pub fn parse_host_style_properties(job: &mut ComponentCompilationJob<'_>) {
    let allocator = job.allocator;

    // Only process the root view (host bindings)
    let root_xref = job.root.xref;

    if let Some(view) = job.view_mut(root_xref) {
        for op in view.update.iter_mut() {
            if let UpdateOp::Binding(binding) = op {
                // Only process property bindings
                if binding.kind != BindingKind::Property {
                    continue;
                }

                let name = binding.name.as_str();

                // Delete any `!important` suffixes from the binding name
                let name = if name.ends_with(BANG_IMPORTANT) {
                    &name[..name.len() - BANG_IMPORTANT.len()]
                } else {
                    name
                };

                // Parse style.* bindings
                if name.starts_with(STYLE_DOT) {
                    binding.kind = BindingKind::StyleProperty;
                    let prop_name = &name[STYLE_DOT.len()..];

                    // Convert camelCase to kebab-case unless it's a CSS custom property
                    let hyphenated = if !is_css_custom_property(prop_name) {
                        hyphenate(prop_name)
                    } else {
                        prop_name.to_string()
                    };

                    // Parse property and unit suffix
                    let (property, unit) = parse_property(&hyphenated);
                    binding.name = Ident::from(allocator.alloc_str(&property));
                    binding.unit = unit.map(|u| Ident::from(allocator.alloc_str(&u)));
                } else if name.starts_with(STYLE_BANG) {
                    binding.kind = BindingKind::StyleProperty;
                    binding.name = Ident::from("style");
                } else if name.starts_with(CLASS_DOT) {
                    binding.kind = BindingKind::ClassName;
                    let class_name = &name[CLASS_DOT.len()..];
                    let (property, _) = parse_property(class_name);
                    binding.name = Ident::from(allocator.alloc_str(&property));
                } else if name.starts_with(CLASS_BANG) {
                    binding.kind = BindingKind::ClassName;
                    let class_name = &name[CLASS_BANG.len()..];
                    let (property, _) = parse_property(class_name);
                    binding.name = Ident::from(allocator.alloc_str(&property));
                }
            }
        }
    }
}

/// Checks whether property name is a custom CSS property (starts with --).
fn is_css_custom_property(name: &str) -> bool {
    name.starts_with("--")
}

/// Converts camelCase to kebab-case.
fn hyphenate(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 5);
    for (i, c) in value.chars().enumerate() {
        if c.is_ascii_uppercase() && i > 0 {
            result.push('-');
            result.push(c.to_ascii_lowercase());
        } else {
            result.push(c);
        }
    }
    result
}

/// Parses a property name, extracting any unit suffix.
///
/// For example: `width.px` → (`width`, Some(`px`))
fn parse_property(name: &str) -> (String, Option<String>) {
    // Remove !important suffix if present
    let name = if let Some(idx) = name.find("!important") {
        if idx > 0 { &name[..idx] } else { "" }
    } else {
        name
    };

    // Check for unit suffix
    if let Some(unit_idx) = name.rfind('.') {
        if unit_idx > 0 {
            let property = name[..unit_idx].to_string();
            let suffix = name[unit_idx + 1..].to_string();
            return (property, Some(suffix));
        }
    }

    (name.to_string(), None)
}

/// Parses host style property bindings for HostBindingCompilationJob.
///
/// This is the host binding version that works with HostBindingCompilationJob.
pub fn parse_host_style_properties_for_host(job: &mut HostBindingCompilationJob<'_>) {
    let allocator = job.allocator;

    for op in job.root.update.iter_mut() {
        if let UpdateOp::Binding(binding) = op {
            // Only process property bindings
            if binding.kind != BindingKind::Property {
                continue;
            }

            let name = binding.name.as_str();

            // Delete any `!important` suffixes from the binding name
            let name = if name.ends_with(BANG_IMPORTANT) {
                &name[..name.len() - BANG_IMPORTANT.len()]
            } else {
                name
            };

            // Parse style.* bindings
            if name.starts_with(STYLE_DOT) {
                binding.kind = BindingKind::StyleProperty;
                let prop_name = &name[STYLE_DOT.len()..];

                // Convert camelCase to kebab-case unless it's a CSS custom property
                let hyphenated = if !is_css_custom_property(prop_name) {
                    hyphenate(prop_name)
                } else {
                    prop_name.to_string()
                };

                // Parse property and unit suffix
                let (property, unit) = parse_property(&hyphenated);
                binding.name = Ident::from(allocator.alloc_str(&property));
                binding.unit = unit.map(|u| Ident::from(allocator.alloc_str(&u)));
            } else if name.starts_with(STYLE_BANG) {
                binding.kind = BindingKind::StyleProperty;
                binding.name = Ident::from("style");
            } else if name.starts_with(CLASS_DOT) {
                binding.kind = BindingKind::ClassName;
                let class_name = &name[CLASS_DOT.len()..];
                let (property, _) = parse_property(class_name);
                binding.name = Ident::from(allocator.alloc_str(&property));
            } else if name.starts_with(CLASS_BANG) {
                binding.kind = BindingKind::ClassName;
                let class_name = &name[CLASS_BANG.len()..];
                let (property, _) = parse_property(class_name);
                binding.name = Ident::from(allocator.alloc_str(&property));
            }
        }
    }
}

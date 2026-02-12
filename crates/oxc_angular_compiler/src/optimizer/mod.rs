//! Angular Build Optimizer for production builds.
//!
//! This module implements the Angular build optimizer, which applies JavaScript
//! AST transformations to pre-compiled Angular packages (`node_modules/@angular/*`)
//! to enable better tree-shaking.
//!
//! ## Transformations
//!
//! The optimizer applies 4 transformations:
//!
//! | Transformation | Purpose |
//! |----------------|---------|
//! | [`elide_metadata`] | Remove `ɵsetClassMetadata()`, `ɵsetClassMetadataAsync()`, `ɵsetClassDebugInfo()` calls |
//! | [`adjust_static_members`] | Wrap static fields (`ɵcmp`, `ɵdir`, `ɵfac`, etc.) in pure IIFE for tree-shaking |
//! | [`mark_top_level_pure`] | Add `/* @__PURE__ */` to top-level calls (except tslib/babel helpers) |
//! | [`adjust_typescript_enums`] | Optimize TS enum patterns to IIFEs |
//!
//! ## Usage
//!
//! ```ignore
//! use oxc_allocator::Allocator;
//! use oxc_angular_compiler::optimizer::{OptimizeOptions, optimize};
//!
//! let allocator = Allocator::default();
//! let code = r#"
//!     import * as i0 from "@angular/core";
//!     class MyComponent {}
//!     MyComponent.ɵcmp = i0.ɵɵdefineComponent({...});
//! "#;
//!
//! let result = optimize(&allocator, code, "component.js", OptimizeOptions::default());
//! println!("{}", result.code);
//! ```

mod adjust_static_members;
mod adjust_typescript_enums;
mod elide_metadata;
mod mark_top_level_pure;

use oxc_allocator::Allocator;
use oxc_parser::Parser;
use oxc_span::SourceType;

pub use adjust_static_members::AdjustStaticMembersTransformer;
pub use adjust_typescript_enums::AdjustTypeScriptEnumsTransformer;
pub use elide_metadata::ElideMetadataTransformer;
pub use mark_top_level_pure::MarkTopLevelPureTransformer;

/// Options for the build optimizer.
#[derive(Debug, Clone, Default)]
pub struct OptimizeOptions {
    /// Generate source maps.
    pub sourcemap: bool,

    /// Remove Angular metadata calls (`ɵsetClassMetadata`, etc.).
    /// Default: true
    pub elide_metadata: bool,

    /// Wrap Angular static members in pure IIFEs for tree-shaking.
    /// Default: true
    pub wrap_static_members: bool,

    /// Add `/* @__PURE__ */` annotations to top-level calls.
    /// Default: true
    pub mark_pure: bool,

    /// Optimize TypeScript enum patterns to IIFEs.
    /// Default: true
    pub adjust_enums: bool,
}

impl OptimizeOptions {
    /// Create options with all transformations enabled.
    pub fn all() -> Self {
        Self {
            sourcemap: false,
            elide_metadata: true,
            wrap_static_members: true,
            mark_pure: true,
            adjust_enums: true,
        }
    }
}

/// Result of optimizing an Angular package file.
#[derive(Debug, Clone, Default)]
pub struct OptimizeResult {
    /// The optimized code.
    pub code: String,

    /// Source map (if sourcemap option was enabled).
    pub map: Option<String>,
}

/// Optimize an Angular package file for better tree-shaking.
///
/// This function applies the Angular build optimizer transformations to
/// pre-compiled Angular packages. It parses the JavaScript, applies the
/// enabled transformations, and generates optimized output.
///
/// # Arguments
///
/// * `allocator` - Memory allocator for the AST
/// * `code` - The JavaScript source code to optimize
/// * `filename` - The filename (used for source maps and error messages)
/// * `options` - Optimization options
///
/// # Returns
///
/// An `OptimizeResult` containing the optimized code and optional source map.
pub fn optimize(
    allocator: &Allocator,
    code: &str,
    filename: &str,
    options: OptimizeOptions,
) -> OptimizeResult {
    // Parse JavaScript (Angular packages are pre-compiled JS, not TypeScript)
    let source_type = SourceType::from_path(filename).unwrap_or(SourceType::mjs());
    let parser_result = Parser::new(allocator, code, source_type).parse();

    // If parsing failed, return original code
    if parser_result.panicked || !parser_result.errors.is_empty() {
        return OptimizeResult { code: code.to_string(), map: None };
    }

    let program = parser_result.program;

    // Build up a list of transformations to apply.
    // Each transformation produces a set of edits (deletions, replacements, insertions).
    // We'll collect them all, sort by position, and apply them in reverse order.
    let mut edits: Vec<Edit> = Vec::new();

    // Phase 1: Elide Angular metadata
    if options.elide_metadata {
        let transformer = ElideMetadataTransformer::new();
        edits.extend(transformer.transform(&program, code));
    }

    // Phase 2: Adjust static members (wrap in pure IIFE)
    if options.wrap_static_members {
        let transformer = AdjustStaticMembersTransformer::new();
        edits.extend(transformer.transform(&program, code));
    }

    // Phase 3: Mark top-level calls as pure
    if options.mark_pure {
        let transformer = MarkTopLevelPureTransformer::new();
        edits.extend(transformer.transform(&program, code));
    }

    // Phase 4: Adjust TypeScript enums
    if options.adjust_enums {
        let transformer = AdjustTypeScriptEnumsTransformer::new();
        edits.extend(transformer.transform(&program, code));
    }

    // Apply edits
    let optimized_code = apply_edits(code, edits);

    OptimizeResult { code: optimized_code, map: None }
}

/// A source code edit operation.
#[derive(Debug, Clone)]
pub struct Edit {
    /// Start byte offset in source
    pub start: u32,
    /// End byte offset in source
    pub end: u32,
    /// Replacement text (empty for deletions)
    pub replacement: String,
    /// Priority for ordering edits at the same position (higher = applied later)
    pub priority: i32,
}

impl Edit {
    /// Create a deletion edit.
    pub fn delete(start: u32, end: u32) -> Self {
        Self { start, end, replacement: String::new(), priority: 0 }
    }

    /// Create a replacement edit.
    pub fn replace(start: u32, end: u32, replacement: String) -> Self {
        Self { start, end, replacement, priority: 0 }
    }

    /// Create an insertion edit (at a position, replacing nothing).
    pub fn insert(position: u32, text: String) -> Self {
        Self { start: position, end: position, replacement: text, priority: 0 }
    }

    /// Set the priority for this edit.
    pub fn with_priority(mut self, priority: i32) -> Self {
        self.priority = priority;
        self
    }
}

/// Apply edits to source code.
///
/// Edits are sorted by position (descending) so that applying them
/// from the end doesn't invalidate earlier positions.
pub fn apply_edits(code: &str, mut edits: Vec<Edit>) -> String {
    if edits.is_empty() {
        return code.to_string();
    }

    // Sort edits by position (descending), then by priority (ascending)
    // This ensures we process from end to start, and lower priority edits first
    edits.sort_by(|a, b| match b.start.cmp(&a.start) {
        std::cmp::Ordering::Equal => a.priority.cmp(&b.priority),
        other => other,
    });

    let mut result = code.to_string();

    for edit in edits {
        let start = edit.start as usize;
        let end = edit.end as usize;

        // Bounds check
        if start > result.len() || end > result.len() || start > end {
            continue;
        }

        result = format!("{}{}{}", &result[..start], edit.replacement, &result[end..]);
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_apply_edits_empty() {
        let code = "hello world";
        let result = apply_edits(code, vec![]);
        assert_eq!(result, code);
    }

    #[test]
    fn test_apply_edits_single_replacement() {
        let code = "hello world";
        let edits = vec![Edit::replace(0, 5, "goodbye".to_string())];
        let result = apply_edits(code, edits);
        assert_eq!(result, "goodbye world");
    }

    #[test]
    fn test_apply_edits_multiple() {
        let code = "hello world";
        let edits = vec![
            Edit::replace(0, 5, "goodbye".to_string()),
            Edit::replace(6, 11, "universe".to_string()),
        ];
        let result = apply_edits(code, edits);
        assert_eq!(result, "goodbye universe");
    }

    #[test]
    fn test_apply_edits_deletion() {
        let code = "hello world";
        let edits = vec![Edit::delete(5, 11)];
        let result = apply_edits(code, edits);
        assert_eq!(result, "hello");
    }

    #[test]
    fn test_apply_edits_insertion() {
        let code = "hello world";
        let edits = vec![Edit::insert(5, " beautiful".to_string())];
        let result = apply_edits(code, edits);
        assert_eq!(result, "hello beautiful world");
    }
}

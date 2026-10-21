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
    if parser_result.fatal_error || !parser_result.diagnostics.is_empty() {
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
    let (optimized_code, map) = if options.sourcemap {
        apply_edits_with_sourcemap(code, edits, filename)
    } else {
        (apply_edits(code, edits), None)
    };

    OptimizeResult { code: optimized_code, map }
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

/// Apply edits to source code and generate a source map.
///
/// Uses the same edit-application algorithm as `apply_edits`, then generates
/// a source map by finding where unchanged source segments appear in the
/// actual output — guaranteeing the sourcemap is consistent with the output
/// regardless of edit ordering.
pub fn apply_edits_with_sourcemap(
    code: &str,
    edits: Vec<Edit>,
    filename: &str,
) -> (String, Option<String>) {
    // Generate the output using the existing algorithm
    let output = apply_edits(code, edits.clone());

    // Generate sourcemap by finding unchanged source segments in the actual output
    let map = generate_sourcemap_from_edits(code, &output, edits, filename);
    (output, Some(map))
}

/// Generate a source map by finding unchanged source segments in the actual output.
///
/// Instead of independently modeling how edits transform positions (which could
/// diverge from `apply_edits`'s reverse-order mutating algorithm), this function:
/// 1. Computes which source byte ranges are untouched by any edit
/// 2. Locates each unchanged segment in the actual output string
/// 3. Generates identity mappings for those segments
///
/// This guarantees the sourcemap is always consistent with the actual output.
fn generate_sourcemap_from_edits(
    source: &str,
    output: &str,
    edits: Vec<Edit>,
    filename: &str,
) -> String {
    let mut builder = oxc_sourcemap::SourceMapBuilder::default();
    builder.set_source_and_content(filename, source);

    if edits.is_empty() {
        // Identity mapping — every line maps 1:1
        add_line_mappings_for_segment(&mut builder, source, 0, 0, 0, 0);
        return builder.into_sourcemap().to_json_string();
    }

    // 1. Collect all edit boundary positions.
    //    Every edit start/end position is a point where the output may differ from
    //    the source. We need to split unchanged ranges at ALL edit positions —
    //    including pure insertions (start == end) — because insertions embed new
    //    text within what would otherwise be a contiguous source segment, breaking
    //    `find(segment)` in step 3.
    let code_len = source.len() as u32;
    let mut boundary_points: Vec<u32> = Vec::new();
    let mut deleted_ranges: Vec<(u32, u32)> = Vec::new();

    for edit in &edits {
        if edit.start > code_len || edit.end > code_len || edit.start > edit.end {
            continue;
        }
        boundary_points.push(edit.start);
        boundary_points.push(edit.end);
        if edit.start < edit.end {
            deleted_ranges.push((edit.start, edit.end));
        }
    }

    boundary_points.push(0);
    boundary_points.push(code_len);
    boundary_points.sort_unstable();
    boundary_points.dedup();

    // Merge overlapping deleted ranges for quick overlap checks
    deleted_ranges.sort_by_key(|r| r.0);
    let mut merged_deleted: Vec<(u32, u32)> = Vec::new();
    for (s, e) in deleted_ranges {
        if let Some(last) = merged_deleted.last_mut() {
            if s <= last.1 {
                last.1 = last.1.max(e);
                continue;
            }
        }
        merged_deleted.push((s, e));
    }

    // 2. Compute unchanged source sub-ranges.
    //    A sub-range [boundary[i], boundary[i+1]) is unchanged if it doesn't
    //    overlap with any deletion range.
    let mut unchanged: Vec<(u32, u32)> = Vec::new();
    for window in boundary_points.windows(2) {
        let (start, end) = (window[0], window[1]);
        if start >= end {
            continue;
        }
        // Check if this sub-range overlaps with any merged deletion
        let overlaps = merged_deleted.iter().any(|(del_s, del_e)| start < *del_e && end > *del_s);
        if !overlaps {
            unchanged.push((start, end));
        }
    }

    // 3. Compute the output byte offset for each unchanged segment and generate mappings.
    //    Instead of using string search (which can false-match replacement text for
    //    short segments like `}`), we compute the exact output position using the
    //    edit shift formula:
    //      output_pos(S) = S + Σ (replacement.len() - (end - start))
    //                      for all edits where end <= S
    //    This is exact for non-overlapping edits.

    // Precompute edit shifts sorted by end position for efficient prefix-sum lookup
    let mut edit_shifts: Vec<(u32, i64)> = edits
        .iter()
        .filter(|e| e.start <= code_len && e.end <= code_len && e.start <= e.end)
        .map(|e| (e.end, e.replacement.len() as i64 - (e.end as i64 - e.start as i64)))
        .collect();
    edit_shifts.sort_by_key(|(end, _)| *end);

    // `unchanged` is in ascending source order, so the edit-shift prefix sum and
    // both line/column cursors only move forward: linear in the input size
    // instead of rescanning from the start for every segment.
    let mut shift_idx = 0;
    let mut net_shift: i64 = 0;
    let mut src_cursor = LineColCursor::new(source);
    let mut out_cursor = LineColCursor::new(output);

    for (src_start, src_end) in &unchanged {
        let segment = &source[*src_start as usize..*src_end as usize];
        if segment.is_empty() {
            continue;
        }
        // Compute output byte offset: src_start + net shift from all edits ending at or before src_start
        while let Some((end, shift)) = edit_shifts.get(shift_idx) {
            if *end > *src_start {
                break;
            }
            net_shift += shift;
            shift_idx += 1;
        }
        let output_byte_pos = (*src_start as i64 + net_shift) as usize;

        debug_assert!(
            output_byte_pos + segment.len() <= output.len()
                && &output[output_byte_pos..output_byte_pos + segment.len()] == segment,
            "Sourcemap: computed output position {output_byte_pos} does not match \
             segment {:?} (src {}..{})",
            &segment[..segment.len().min(20)],
            src_start,
            src_end,
        );

        let (src_line, src_col) = src_cursor.line_col(*src_start as usize);
        let (out_line, out_col) = out_cursor.line_col(output_byte_pos);
        add_line_mappings_for_segment(&mut builder, segment, out_line, out_col, src_line, src_col);
    }

    builder.into_sourcemap().to_json_string()
}

/// Computes line and column (UTF-16 code units) for byte offsets in a string.
///
/// Source map columns must be in UTF-16 code units per the spec and `oxc_sourcemap`
/// convention. For ASCII this equals byte offset; for multi-byte characters
/// (e.g., `ɵ` U+0275 = 2 UTF-8 bytes but 1 UTF-16 code unit) the values differ.
///
/// The cursor remembers where the previous lookup stopped, so ascending lookups
/// scan the string once in total. A lookup behind the cursor restarts from the
/// beginning.
struct LineColCursor<'a> {
    text: &'a str,
    /// Byte offset of the next unscanned character (always a char boundary).
    offset: usize,
    line: u32,
    col: u32,
}

impl<'a> LineColCursor<'a> {
    fn new(text: &'a str) -> Self {
        Self { text, offset: 0, line: 0, col: 0 }
    }

    /// Line and column after every character that starts before `offset`.
    fn line_col(&mut self, offset: usize) -> (u32, u32) {
        if offset < self.offset {
            *self = Self::new(self.text);
        }
        for ch in self.text[self.offset..].chars() {
            if self.offset >= offset {
                break;
            }
            if ch == '\n' {
                self.line += 1;
                self.col = 0;
            } else {
                self.col += ch.len_utf16() as u32;
            }
            self.offset += ch.len_utf8();
        }
        (self.line, self.col)
    }
}

/// Add source map mappings for an unchanged segment of source code.
///
/// Adds a mapping at the start of the segment and at the beginning of each new line.
fn add_line_mappings_for_segment(
    builder: &mut oxc_sourcemap::SourceMapBuilder,
    segment: &str,
    mut out_line: u32,
    mut out_col: u32,
    mut src_line: u32,
    mut src_col: u32,
) {
    // Add mapping at the start of this segment
    builder.add_token(out_line, out_col, src_line, src_col, Some(0), None);

    for ch in segment.chars() {
        if ch == '\n' {
            out_line += 1;
            out_col = 0;
            src_line += 1;
            src_col = 0;
            // Add mapping at the start of each new line
            builder.add_token(out_line, out_col, src_line, src_col, Some(0), None);
        }
    }
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

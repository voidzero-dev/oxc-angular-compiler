//! Attach source locations phase.
//!
//! Attaches debug source location information to operations. When enabled,
//! this phase collects the source locations of all elements in the template
//! and generates `ɵɵsourceLocation` calls that expose this information
//! in the DOM for debugging purposes.
//!
//! Ported from Angular's `template/pipeline/src/phases/attach_source_locations.ts`.

use oxc_str::Ident;

use crate::ir::ops::{CreateOp, CreateOpBase, SourceLocationOp, XrefId};
use crate::pipeline::compilation::ComponentCompilationJob;

/// Collected element source location for debugging.
#[derive(Debug, Clone)]
struct ElementSourceLocation {
    /// Target element's xref ID.
    target: XrefId,
    /// Line number (1-indexed).
    line: u32,
    /// Column number (0-indexed).
    column: u32,
}

/// Computes line (1-indexed) and column (0-indexed) from a byte offset.
fn offset_to_line_column(source: &str, offset: u32) -> (u32, u32) {
    let offset = offset as usize;
    let bytes = source.as_bytes();

    let mut line = 1u32;
    let mut col = 0u32;

    for (i, &byte) in bytes.iter().enumerate() {
        if i >= offset {
            break;
        }
        if byte == b'\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }

    (line, col)
}

/// Attaches source location information for debugging.
///
/// This phase collects element definitions from the create block and
/// outputs operations that expose their source locations in the DOM.
/// This is only enabled when `job.enable_debug_locations` is true and
/// `job.relative_template_path` is set.
///
/// The collected information includes:
/// - Target xref of the element
/// - Source offset, line, and column
pub fn attach_source_locations(job: &mut ComponentCompilationJob<'_>) {
    // Early return if debug locations are disabled or path is not set
    if !job.enable_debug_locations {
        return;
    }
    let template_path = match &job.relative_template_path {
        Some(path) => path.clone(),
        None => return,
    };
    let template_source = match job.template_source {
        Some(source) => source,
        None => return,
    };

    // Collect view xrefs to avoid borrow issues
    let view_xrefs: Vec<_> = job.all_views().map(|v| v.xref).collect();

    for view_xref in view_xrefs {
        if let Some(view) = job.view_mut(view_xref) {
            // Collect element source locations from create ops
            let mut locations: Vec<ElementSourceLocation> = Vec::new();

            for op in view.create.iter() {
                match op {
                    CreateOp::ElementStart(elem) => {
                        if let Some(span) = elem.base.source_span {
                            let (line, column) = offset_to_line_column(template_source, span.start);
                            locations.push(ElementSourceLocation {
                                target: elem.xref,
                                line,
                                column,
                            });
                        }
                    }
                    CreateOp::Element(elem) => {
                        if let Some(span) = elem.base.source_span {
                            let (line, column) = offset_to_line_column(template_source, span.start);
                            locations.push(ElementSourceLocation {
                                target: elem.xref,
                                line,
                                column,
                            });
                        }
                    }
                    _ => {}
                }
            }

            // Create SourceLocationOp for each element with location info
            // Note: Angular creates a single op with an array, but our IR uses per-element ops
            for loc in locations {
                let source_op = CreateOp::SourceLocation(SourceLocationOp {
                    base: CreateOpBase::default(),
                    target: loc.target,
                    template_url: Ident::from(template_path.as_str()),
                    line: loc.line,
                    column: loc.column,
                });
                view.create.push(source_op);
            }
        }
    }
}

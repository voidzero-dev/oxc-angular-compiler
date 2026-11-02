//! Attach source locations phase.
//!
//! With Angular's `enableTemplateSourceLocations` option, each view's create
//! block ends with one `ɵɵattachSourceLocations(path, [[slot, offset, line,
//! column], ...])` call that lists where each of its elements starts in the
//! source. The runtime writes it into a `data-ng-source-location` attribute.
//!
//! Ported from Angular's `template/pipeline/src/phases/attach_source_locations.ts`.

use crate::ir::ops::{CreateOp, CreateOpBase, SourceLocation, SourceLocationOp};
use crate::pipeline::compilation::ComponentCompilationJob;
use oxc_allocator::Vec as OxcVec;

/// The text Angular's template lexer reads a template from, so element
/// positions can be given the way ngtsc gives them.
///
/// ngtsc (`extractTemplate` in compiler-cli's component resources) lexes an
/// inline string or no-substitution template literal straight out of the
/// `.ts` source, so its positions are those of the raw source. Any other
/// template (a `templateUrl` file, or a value read from a variable) is lexed
/// on its own, from position 0.
#[derive(Debug, Clone, Copy)]
pub struct TemplateSource<'a> {
    /// The text the positions count in: the whole `.ts` source for an inline
    /// literal, the template itself otherwise.
    pub text: &'a str,
    /// The byte offset in `text` where the template starts.
    pub start: u32,
    /// The 0-based line and UTF-16 column of `start`, from TypeScript's
    /// `getLineAndCharacterOfPosition` (ngtsc's `getTemplateRange`).
    pub start_line: u32,
    pub start_column: u32,
    /// For an inline literal written with escapes, the byte offset in `text`
    /// of each byte of the cooked template. `None` when the cooked template
    /// is the raw text from `start` on.
    pub raw_offsets: Option<&'a [u32]>,
}

impl TemplateSource<'_> {
    /// The UTF-16 offset, 0-based line and 0-based UTF-16 column of each byte
    /// offset in the cooked template, in the order given.
    ///
    /// Line and column are counted the way Angular's lexer cursor
    /// (`PlainCharacterCursor.advanceState`) counts them over the raw text:
    /// `\n` starts a line, `\r` moves nothing, and any other UTF-16 unit
    /// moves one column. An escape sequence is counted by its raw characters,
    /// and a character it stands for sits at its backslash.
    fn locate(&self, offsets: &[u32]) -> std::vec::Vec<(u32, u32, u32)> {
        let raw = |offset: u32| match self.raw_offsets {
            Some(map) => map.get(offset as usize).copied().unwrap_or(self.start),
            None => self.start + offset,
        } as usize;

        // One forward pass over the text, visiting the offsets in order.
        let mut order: std::vec::Vec<usize> = (0..offsets.len()).collect();
        order.sort_unstable_by_key(|&i| raw(offsets[i]));
        let mut result = vec![(0, 0, 0); offsets.len()];

        let start = self.start as usize;
        let mut pos = start;
        let mut utf16 =
            u32::try_from(self.text[..start].encode_utf16().count()).unwrap_or(u32::MAX);
        let (mut line, mut column) = (self.start_line, self.start_column);
        for i in order {
            let target = raw(offsets[i]).min(self.text.len());
            if target > pos {
                for c in self.text[pos..target].chars() {
                    let units = utf16_len(c);
                    utf16 += units;
                    match c {
                        '\n' => {
                            line += 1;
                            column = 0;
                        }
                        '\r' => {}
                        _ => column += units,
                    }
                }
                pos = target;
            }
            result[i] = (utf16, line, column);
        }
        result
    }
}

/// The UTF-16 units `c` takes: two above the Basic Multilingual Plane.
pub fn utf16_len(c: char) -> u32 {
    if u32::from(c) > 0xFFFF { 2 } else { 1 }
}

/// Attaches source location information for debugging.
///
/// For each view, collects the element ops of the create block and pushes a
/// single `SourceLocation` op at its end, as Angular's `attachSourceLocations`
/// does. It only runs when `job.enable_debug_locations` is true and both the
/// template path and the template source are known.
pub fn attach_source_locations(job: &mut ComponentCompilationJob<'_>) {
    if !job.enable_debug_locations {
        return;
    }
    let (Some(template_path), Some(source)) = (job.relative_template_path, job.template_source)
    else {
        return;
    };
    let allocator = job.allocator;

    let view_xrefs: std::vec::Vec<_> = job.all_views().map(|v| v.xref).collect();
    for view_xref in view_xrefs {
        let Some(view) = job.view_mut(view_xref) else { continue };

        // Angular reads `op.startSourceSpan.start` of every `ElementStart` and
        // `Element` op, and the slot the op was given.
        let mut elements: std::vec::Vec<(u32, u32)> = std::vec::Vec::new();
        for op in view.create.iter() {
            let (slot, span) = match op {
                CreateOp::ElementStart(elem) => (elem.slot, elem.base.source_span),
                CreateOp::Element(elem) => (elem.slot, elem.base.source_span),
                _ => continue,
            };
            if let (Some(slot), Some(span)) = (slot, span) {
                elements.push((slot.0, span.start));
            }
        }
        if elements.is_empty() {
            continue;
        }

        let offsets: std::vec::Vec<u32> = elements.iter().map(|&(_, offset)| offset).collect();
        let positions = source.locate(&offsets);
        let mut locations = OxcVec::with_capacity_in(elements.len(), &allocator);
        for (&(slot, _), (offset, line, column)) in elements.iter().zip(positions) {
            locations.push(SourceLocation { slot, offset, line, column });
        }
        view.create.push(CreateOp::SourceLocation(SourceLocationOp {
            base: CreateOpBase::default(),
            template_path,
            locations,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::TemplateSource;

    fn source(text: &str, start: u32) -> TemplateSource<'_> {
        TemplateSource { text, start, start_line: 0, start_column: 0, raw_offsets: None }
    }

    #[test]
    fn counts_lines_like_the_angular_lexer() {
        // `\r` moves no column; `\n` starts a line.
        let text = "<div>\r\n  <span>\r<b>";
        let positions = source(text, 0).locate(&[0, 9, 16]);
        assert_eq!(positions, vec![(0, 0, 0), (9, 1, 2), (16, 1, 8)]);
    }

    #[test]
    fn counts_utf16_units() {
        // `é` is one UTF-16 unit, `😀` two; both take more UTF-8 bytes.
        let text = "é😀<i>";
        let offset = u32::try_from(text.find("<i>").unwrap()).unwrap();
        assert_eq!(source(text, 0).locate(&[offset]), vec![(3, 0, 3)]);
    }

    #[test]
    fn keeps_the_given_order() {
        let text = "<a></a>\n<b></b>";
        assert_eq!(source(text, 0).locate(&[8, 0]), vec![(8, 1, 0), (0, 0, 0)]);
    }
}

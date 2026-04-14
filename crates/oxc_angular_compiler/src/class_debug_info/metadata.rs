//! Class debug info structures.
//!
//! Ported from Angular's `render3/r3_class_debug_info_compiler.ts`.

use oxc_str::Ident;

use crate::output::ast::OutputExpression;

/// Info needed for runtime errors related to a class, such as the location
/// in which the class is defined.
///
/// Corresponds to Angular's `R3ClassDebugInfo` interface.
#[derive(Debug)]
pub struct R3ClassDebugInfo<'a> {
    /// The class type for which debug info is captured.
    pub r#type: OutputExpression<'a>,

    /// The original class name as it appears in its definition.
    pub class_name: Ident<'a>,

    /// The relative path of the file in which the class is defined.
    ///
    /// The path is relative to the project root. For security reasons,
    /// absolute file paths are never shown. If the relative path cannot
    /// be computed, this should be `None`, and downstream consumers will
    /// typically ignore the `line_number` field as well.
    pub file_path: Option<Ident<'a>>,

    /// The line number in which this class is defined (1-indexed).
    pub line_number: u32,

    /// Whether to check if this component is being rendered without its
    /// NgModule being loaded into the browser. Such checks are only
    /// carried out in dev mode.
    pub forbid_orphan_rendering: bool,
}

impl<'a> R3ClassDebugInfo<'a> {
    /// Creates a new `R3ClassDebugInfo` with the given type and class name.
    ///
    /// File path and line number default to `None`/`0`, and
    /// `forbid_orphan_rendering` defaults to `false`.
    pub fn new(r#type: OutputExpression<'a>, class_name: Ident<'a>) -> Self {
        Self { r#type, class_name, file_path: None, line_number: 0, forbid_orphan_rendering: false }
    }

    /// Sets the file path for this debug info.
    pub fn with_file_path(mut self, file_path: Ident<'a>) -> Self {
        self.file_path = Some(file_path);
        self
    }

    /// Sets the line number for this debug info.
    pub fn with_line_number(mut self, line_number: u32) -> Self {
        self.line_number = line_number;
        self
    }

    /// Sets whether orphan rendering should be forbidden.
    pub fn with_forbid_orphan_rendering(mut self, forbid: bool) -> Self {
        self.forbid_orphan_rendering = forbid;
        self
    }
}

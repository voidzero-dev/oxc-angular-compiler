//! Class debug info compilation module.
//!
//! Ported from Angular's `render3/r3_class_debug_info_compiler.ts`.
//!
//! Generates class debug information for runtime error messages:
//! ```javascript
//! (() => {
//!   (typeof ngDevMode === "undefined" || ngDevMode) &&
//!     i0.ɵsetClassDebugInfo(MyComponent, {
//!       className: "MyComponent",
//!       filePath: "path/to/file.ts",
//!       lineNumber: 10
//!     });
//! })();
//! ```

mod compiler;
mod metadata;

pub use compiler::compile_class_debug_info;
pub use metadata::{R3ClassDebugInfo, class_name_pos, line_number_at};

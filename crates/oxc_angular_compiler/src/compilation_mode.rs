//! Compilation mode for Angular decorators.
//!
//! Mirrors Angular's `CompilationMode` enum at
//! `packages/compiler-cli/src/ngtsc/transform/src/api.ts:25-41`.
//!
//! - `Full`: emit fully-resolved Ivy definitions (`ɵɵdefineComponent`,
//!   `ɵɵdefineDirective`, …). Used for application builds.
//! - `Partial`: emit partial declarations (`ɵɵngDeclareComponent`,
//!   `ɵɵngDeclareDirective`, …). Used for library builds; consumers run the
//!   linker to expand them.
//!
//! The OXC compiler currently only implements `Full`. `Partial` is being
//! added incrementally, decorator by decorator.

/// Selects between full Ivy emit and partial-declaration emit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CompilationMode {
    /// Emit `ɵɵdefine*` calls — applications and JIT-paired output.
    #[default]
    Full,
    /// Emit `ɵɵngDeclare*` calls — library output for the linker to expand.
    Partial,
}

impl CompilationMode {
    /// Returns true if this is partial-declaration emit mode.
    pub fn is_partial(self) -> bool {
        matches!(self, Self::Partial)
    }
}

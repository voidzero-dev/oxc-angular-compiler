//! Partial-declaration emit for library compilation.
//!
//! Ported from Angular's `packages/compiler/src/render3/partial/`.
//!
//! Each submodule emits one `ɵɵngDeclare*` shape. The linker
//! (`crate::linker`) is the inverse — it expands these calls into the
//! corresponding `ɵɵdefine*` calls at consumer build time.
//!
//! Currently implemented:
//! - `factory` — `ɵɵngDeclareFactory`
//! - `injectable` — `ɵɵngDeclareInjectable`. Wired into the Injectable
//!   emit path.
//! - `pipe` — `ɵɵngDeclarePipe`. Wired into the Pipe emit path.
//! - `ng_module` + `injector` — `ɵɵngDeclareNgModule` and
//!   `ɵɵngDeclareInjector`. Both emit per `@NgModule`; wired into the
//!   NgModule emit path. Partial mode banishes `setNgModuleScope`
//!   (matches upstream `ng_module/handler.ts:971`).
//! - `directive` — `ɵɵngDeclareDirective`. Wired into the Directive emit
//!   path. Inputs auto-select new (post-17.1) vs legacy shape based on
//!   signal-input presence; minVersion bumps to 16.1.0 (transform fn),
//!   17.1.0 (signal input), 17.2.0 (signal query).
//! - `component` — `ɵɵngDeclareComponent`. Wired in at
//!   `component/transform.rs::compile_component_full` — partial mode
//!   takes an early-return path that bypasses the entire template/IR
//!   pipeline. Templates are emitted as verbatim string literals;
//!   control-flow block syntax in the template bumps minVersion to
//!   17.0.0.
//!
//! Setting `TransformOptions.compilation_mode = Partial` on a source
//! containing the above decorators produces fully partial-form output.
//!
//! Not yet implemented (and the dispatch from the per-decorator emit paths
//! falls back to full mode): classMetadata.

pub mod component;
pub mod directive;
pub mod factory;
pub mod injectable;
pub mod injector;
pub mod ng_module;
pub mod pipe;

pub use component::{PartialComponentInputs, compile_declare_component_from_metadata};
pub use directive::compile_declare_directive_from_metadata;
pub use factory::compile_declare_factory_function;
pub use injectable::compile_declare_injectable_from_metadata;
pub use injector::compile_declare_injector_from_metadata;
pub use ng_module::compile_declare_ng_module_from_metadata;
pub use pipe::compile_declare_pipe_from_metadata;

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use crate::output::ast::{
    FunctionExpr, InvokeFunctionExpr, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
    ReturnStatement,
};
use crate::r3::Identifiers;

/// Wraps a forward-referenced expression as `i0.forwardRef(function() { return X; })`.
///
/// Mirrors upstream `generateForwardRef` at
/// `packages/compiler/src/render3/util.ts:174`. The linker recognizes this
/// exact shape and unwraps it before forwarding to the full-mode emitter.
pub(crate) fn wrap_forward_ref<'a>(
    allocator: &'a Allocator,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let mut body: Vec<'a, OutputStatement<'a>> = Vec::new_in(allocator);
    body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: expr, source_span: None },
        allocator,
    )));

    let inner_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: None,
            params: Vec::new_in(allocator),
            statements: body,
            source_span: None,
        },
        allocator,
    ));

    let i0 = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from("i0"), source_span: None },
        allocator,
    ));
    let forward_ref_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(i0, allocator),
            name: Ident::from(Identifiers::FORWARD_REF),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    let mut args = Vec::new_in(allocator);
    args.push(inner_fn);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(forward_ref_fn, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// The literal text used for the `version` field of every partial
/// declaration. Upstream substitutes this at npm publish; we keep the same
/// sentinel so the existing linker version-check logic recognizes it.
///
/// Matches upstream `packages/compiler/src/render3/partial/*.ts` — every
/// emitter writes `"0.0.0-PLACEHOLDER"` directly.
pub const PLACEHOLDER_VERSION: &str = "0.0.0-PLACEHOLDER";

/// Minimum linker version that understands a `ɵɵngDeclareFactory` shape.
///
/// Matches upstream `packages/compiler/src/render3/partial/factory.ts:25`.
pub const MIN_VERSION_FACTORY: &str = "12.0.0";

/// Minimum linker version for the other partial-declaration kinds.
///
/// These constants are used as new partial emitters land. Listed here so
/// every minVersion lives in one place and stays in sync with upstream.
#[allow(dead_code)]
pub(crate) const MIN_VERSION_INJECTABLE: &str = "12.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_INJECTOR: &str = "12.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_CLASS_METADATA: &str = "12.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_PIPE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_NG_MODULE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_DIRECTIVE_BASE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_COMPONENT_BASE: &str = "14.0.0";
#[allow(dead_code)]
pub(crate) const MIN_VERSION_CLASS_METADATA_ASYNC: &str = "18.0.0";

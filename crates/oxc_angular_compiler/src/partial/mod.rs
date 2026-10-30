//! Partial-declaration emit for library compilation.
//!
//! Ported from Angular's `packages/compiler/src/render3/partial/`.
//!
//! Each submodule emits one `ɵɵngDeclare*` shape. The linker
//! (`crate::linker`) is the inverse — it expands these calls into the
//! corresponding `ɵɵdefine*` calls at consumer build time.
//!
//! Implemented:
//! - `factory` — `ɵɵngDeclareFactory`
//! - `injectable` — `ɵɵngDeclareInjectable`
//! - `service` — `ɵɵngDeclareService` (Angular v22+ `@Service`)
//! - `pipe` — `ɵɵngDeclarePipe`
//! - `ng_module` + `injector` — `ɵɵngDeclareNgModule` and
//!   `ɵɵngDeclareInjector`. Partial mode banishes `setNgModuleScope`
//!   (matches upstream `ng_module/handler.ts:971`).
//! - `directive` — `ɵɵngDeclareDirective`. Inputs auto-select new
//!   (post-17.1) vs legacy shape based on signal-input presence;
//!   minVersion bumps to 16.1.0 (transform fn), 17.1.0 (signal input),
//!   17.2.0 (signal query).
//! - `component` — `ɵɵngDeclareComponent`. Wired in at
//!   `component/transform.rs::compile_component_full` — partial mode
//!   takes an early-return path that bypasses the entire template/IR
//!   pipeline. Templates are emitted as verbatim string literals;
//!   control-flow block syntax in the template bumps minVersion to
//!   17.0.0.
//! - `class_metadata` — sync `ɵɵngDeclareClassMetadata` (replaces the
//!   full-mode `(() => { (typeof ngDevMode === "undefined" || ngDevMode)
//!   && i0.ɵsetClassMetadata(...); })();` IIFE) AND async
//!   `ɵɵngDeclareClassMetadataAsync` for components with `@defer`
//!   deferrable imports. The dispatch helper
//!   `compile_component_declare_class_metadata` picks between them
//!   based on whether `R3DeferPerComponentDependency` entries are
//!   present.
//!
//! Setting `TransformOptions.compilation_mode = Partial` on a source
//! containing any of these decorators produces fully partial-form
//! output that the existing linker (`crate::linker`) expands back into
//! valid full Ivy form.

pub mod class_metadata;
pub mod component;
pub mod directive;
pub mod factory;
pub mod injectable;
pub mod injector;
pub mod ng_module;
pub mod pipe;
pub mod service;

pub use class_metadata::{
    compile_component_declare_class_metadata, compile_declare_class_metadata,
    compile_declare_class_metadata_async,
};
pub use component::{PartialComponentInputs, compile_declare_component_from_metadata};
pub use directive::compile_declare_directive_from_metadata;
pub use factory::compile_declare_factory_function;
pub use injectable::compile_declare_injectable_from_metadata;
pub use injector::compile_declare_injector_from_metadata;
pub use ng_module::compile_declare_ng_module_from_metadata;
pub use pipe::compile_declare_pipe_from_metadata;
pub use service::compile_declare_service_from_metadata;

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
    core_namespace: &'a str,
    expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let mut body: Vec<'a, OutputStatement<'a>> = Vec::new_in(&allocator);
    body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: expr, source_span: None },
        &allocator,
    )));

    let inner_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: None,
            params: Vec::new_in(&allocator),
            statements: body,
            source_span: None,
        },
        &allocator,
    ));

    let i0 = OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: Ident::from(core_namespace), source_span: None },
        &allocator,
    ));
    let forward_ref_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(i0, &allocator),
            name: Ident::from(Identifiers::FORWARD_REF),
            optional: false,
            source_span: None,
        },
        &allocator,
    ));

    let mut args = Vec::new_in(&allocator);
    args.push(inner_fn);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(forward_ref_fn, &allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        &allocator,
    ))
}

/// The `version` field ngtsc's own development builds write.
///
/// npm publishing replaces it with the release version, and a published
/// linker reads it as `0.0.0`, which turns off the template syntax and
/// defaults added since (block syntax, standalone, `OnPush`), so oxc writes a
/// real version instead (see [`declaration_version`]).
pub const PLACEHOLDER_VERSION: &str = "0.0.0-PLACEHOLDER";

/// The newest Angular release oxc's output follows: what a compilation
/// without a known `angular_version` targets.
pub const LATEST_VERSION: &str = "22.2.3";

/// The `version` field of a partial declaration, which the linker reads its
/// defaults by: the Angular version compiled for, as ngtsc writes its own
/// version, else [`LATEST_VERSION`].
pub fn declaration_version(angular_version: Option<crate::AngularVersion>) -> String {
    match angular_version {
        // `AngularVersion::parse` reads the placeholder as the newest version.
        Some(version) if version.major != u32::MAX => {
            format!("{}.{}.{}", version.major, version.minor, version.patch)
        }
        _ => LATEST_VERSION.to_string(),
    }
}

/// Whether the linker reads a declaration of `version` without
/// `isStandalone` as standalone: from v19 on, as Angular's linker's
/// `getDefaultStandaloneValue` does.
pub(crate) fn standalone_is_default(version: &str) -> bool {
    version
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .is_some_and(|major| major >= 19)
}

/// Minimum linker version that understands a `ɵɵngDeclareFactory` shape.
///
/// Matches upstream `packages/compiler/src/render3/partial/factory.ts:25`.
pub const MIN_VERSION_FACTORY: &str = "12.0.0";

/// Minimum linker version for the other partial-declaration kinds.
///
/// Listed here so every minVersion lives in one place and stays in sync
/// with upstream's `packages/compiler/src/render3/partial/*.ts` files.
/// Directive and Component compute their effective minVersion dynamically
/// (signal inputs, signal queries, control-flow blocks all bump it) so no
/// dedicated DIRECTIVE_BASE / COMPONENT_BASE constant is needed here.
pub(crate) const MIN_VERSION_INJECTABLE: &str = "12.0.0";
pub(crate) const MIN_VERSION_INJECTOR: &str = "12.0.0";
pub(crate) const MIN_VERSION_CLASS_METADATA: &str = "12.0.0";
pub(crate) const MIN_VERSION_PIPE: &str = "14.0.0";
pub(crate) const MIN_VERSION_NG_MODULE: &str = "14.0.0";
pub(crate) const MIN_VERSION_CLASS_METADATA_ASYNC: &str = "18.0.0";
pub(crate) const MIN_VERSION_SERVICE: &str = "22.0.0";

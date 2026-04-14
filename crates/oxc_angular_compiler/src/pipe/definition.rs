//! Pipe definition generation (ɵpipe and ɵfac).
//!
//! This module generates the Angular runtime definitions that are added
//! as static properties on pipe classes:
//!
//! - `ɵpipe`: Pipe definition created by `ɵɵdefinePipe()`
//! - `ɵfac`: Factory function for dependency injection (when the pipe has constructor deps)
//!
//! ## Generated Output
//!
//! ```javascript
//! MyPipe.ɵpipe = /*@__PURE__*/ i0.ɵɵdefinePipe({
//!   name: "myPipe",
//!   type: MyPipe,
//!   pure: true
//! });
//! MyPipe.ɵfac = function MyPipe_Factory(__ngFactoryType__) {
//!   return new (__ngFactoryType__ || MyPipe)(i0.ɵɵdirectiveInject(SomeService));
//! };
//! ```
//!
//! Note: When `standalone` is true (the default in Angular v19+), it is omitted
//! from the output. When `standalone: false`, it is explicitly included.

use oxc_allocator::{Allocator, Vec as OxcVec};

use super::compiler::compile_pipe;
use super::decorator::PipeMetadata;
use super::metadata::R3PipeMetadata;
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata, compile_factory_function,
};
use crate::output::ast::{OutputExpression, ReadVarExpr};

/// Result of generating pipe definition (ɵpipe only).
///
/// This is the legacy struct that only includes the pipe definition.
/// Use `FullPipeDefinition` for complete generation including factory.
pub struct PipeDefinition<'a> {
    /// The ɵpipe definition (pipe metadata for Angular runtime).
    /// This is the result of `ɵɵdefinePipe({...})`.
    pub pipe_definition: OutputExpression<'a>,
}

/// Full result of generating pipe definitions (ɵpipe and ɵfac).
///
/// Pipes with constructor dependencies need both definitions:
/// - `ɵpipe`: The pipe definition for Angular's pipe system
/// - `ɵfac`: The factory function for dependency injection
pub struct FullPipeDefinition<'a> {
    /// The ɵpipe definition (pipe metadata for Angular runtime).
    pub pipe_definition: OutputExpression<'a>,

    /// The ɵfac factory function definition.
    /// This handles dependency injection when the pipe has constructor parameters.
    pub fac_definition: OutputExpression<'a>,
}

/// Generate ɵpipe definition for a pipe.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Pipe R3 metadata (already converted from decorator metadata)
///
/// # Returns
///
/// The ɵpipe definition as an output expression.
///
/// # Example Output
///
/// ```javascript
/// // Pure standalone pipe (default in v19+):
/// MyPipe.ɵpipe = /*@__PURE__*/ i0.ɵɵdefinePipe({
///   name: "myPipe",
///   type: MyPipe,
///   pure: true
/// });
///
/// // Impure pipe:
/// ImpurePipe.ɵpipe = /*@__PURE__*/ i0.ɵɵdefinePipe({
///   name: "impure",
///   type: ImpurePipe,
///   pure: false
/// });
///
/// // Non-standalone pipe (for v18 and earlier or explicit standalone: false):
/// LegacyPipe.ɵpipe = /*@__PURE__*/ i0.ɵɵdefinePipe({
///   name: "legacy",
///   type: LegacyPipe,
///   pure: true,
///   standalone: false
/// });
/// ```
pub fn generate_pipe_definition<'a>(
    allocator: &'a Allocator,
    metadata: &R3PipeMetadata<'a>,
) -> PipeDefinition<'a> {
    let result = compile_pipe(allocator, metadata);
    PipeDefinition { pipe_definition: result.expression }
}

/// Generate ɵpipe definition directly from pipe decorator metadata.
///
/// This is a convenience function that converts decorator metadata to R3 metadata
/// and generates the definition in one step.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Pipe metadata extracted from `@Pipe` decorator
///
/// # Returns
///
/// `Some(PipeDefinition)` if conversion and compilation succeeded,
/// `None` if the metadata couldn't be converted to R3 format.
pub fn generate_pipe_definition_from_decorator<'a>(
    allocator: &'a Allocator,
    metadata: &PipeMetadata<'a>,
) -> Option<PipeDefinition<'a>> {
    let r3_metadata = metadata.to_r3_metadata(allocator)?;
    Some(generate_pipe_definition(allocator, &r3_metadata))
}

/// Generate both ɵpipe and ɵfac definitions from pipe decorator metadata.
///
/// This is the complete generation function that creates both:
/// - The pipe definition (`ɵpipe`) for Angular's pipe system
/// - The factory function (`ɵfac`) for dependency injection
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Pipe metadata extracted from `@Pipe` decorator
///
/// # Returns
///
/// `Some(FullPipeDefinition)` if compilation succeeded,
/// `None` if the metadata couldn't be converted to R3 format.
pub fn generate_full_pipe_definition_from_decorator<'a>(
    allocator: &'a Allocator,
    metadata: &PipeMetadata<'a>,
) -> Option<FullPipeDefinition<'a>> {
    let r3_metadata = metadata.to_r3_metadata(allocator)?;
    // IMPORTANT: Generate ɵfac BEFORE ɵpipe to match Angular's namespace index assignment order.
    // Angular processes results in order [fac, def, ...] during the transform phase
    // (see packages/compiler-cli/src/ngtsc/annotations/src/pipe.ts:266-273),
    // so factory dependencies get registered first, followed by pipe definition dependencies.
    // This ensures namespace indices (i0, i1, i2, ...) are assigned in the same order.
    let fac_definition = generate_pipe_fac(allocator, metadata);
    let pipe_result = compile_pipe(allocator, &r3_metadata);

    Some(FullPipeDefinition { pipe_definition: pipe_result.expression, fac_definition })
}

/// Generate ɵfac factory function for a pipe.
fn generate_pipe_fac<'a>(
    allocator: &'a Allocator,
    metadata: &PipeMetadata<'a>,
) -> OutputExpression<'a> {
    let factory_name = allocator.alloc_str(&format!("{}_Factory", metadata.class_name));

    let type_expr = OutputExpression::ReadVar(oxc_allocator::Box::new_in(
        ReadVarExpr { name: metadata.class_name.clone(), source_span: None },
        allocator,
    ));

    // Convert deps from PipeMetadata format to R3FactoryDeps
    let factory_deps = match &metadata.deps {
        Some(deps) => {
            let mut factory_deps: OxcVec<'a, R3DependencyMetadata<'a>> =
                OxcVec::with_capacity_in(deps.len(), allocator);
            for dep in deps {
                factory_deps.push(R3DependencyMetadata {
                    token: dep.token.as_ref().map(|t| t.clone_in(allocator)),
                    attribute_name_type: dep
                        .attribute_name_type
                        .as_ref()
                        .map(|a| a.clone_in(allocator)),
                    host: dep.host,
                    optional: dep.optional,
                    self_: dep.self_,
                    skip_self: dep.skip_self,
                });
            }
            R3FactoryDeps::Valid(factory_deps)
        }
        None => R3FactoryDeps::None,
    };

    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: metadata.class_name.clone(),
        type_expr: type_expr.clone_in(allocator),
        type_decl: type_expr,
        type_argument_count: 0,
        deps: factory_deps,
        target: FactoryTarget::Pipe,
    });

    let result = compile_factory_function(allocator, &factory_meta, factory_name);
    result.expression
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::ReadVarExpr;
    use crate::output::emitter::JsEmitter;
    use crate::pipe::metadata::R3PipeMetadataBuilder;
    use oxc_allocator::Box;
    use oxc_str::Ident;

    #[test]
    fn test_generate_pure_pipe_definition() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyPipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadataBuilder::new(Ident::from("MyPipe"), type_expr)
            .pipe_name(Ident::from("myPipe"))
            .pure(true)
            .is_standalone(true)
            .build();

        let definition = generate_pipe_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.pipe_definition);

        // Should have definePipe call
        assert!(js.contains("ɵɵdefinePipe"), "Should contain ɵɵdefinePipe");
        // Should have pipe name
        assert!(js.contains("myPipe"), "Should contain pipe name 'myPipe'");
        // Should have type
        assert!(js.contains("type"), "Should contain type property");
        assert!(js.contains("MyPipe"), "Should reference MyPipe type");
        // Should have pure: true
        assert!(js.contains("pure"), "Should contain pure property");
        assert!(js.contains("true"), "Should have pure: true");
        // Should NOT have standalone when it's true (it's the default)
        assert!(!js.contains("standalone"), "Should NOT contain standalone when it's true");
    }

    #[test]
    fn test_generate_impure_pipe_definition() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ImpurePipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadataBuilder::new(Ident::from("ImpurePipe"), type_expr)
            .pipe_name(Ident::from("impure"))
            .pure(false)
            .is_standalone(true)
            .build();

        let definition = generate_pipe_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.pipe_definition);

        // Should have pure: false
        assert!(js.contains("pure"), "Should contain pure property");
        assert!(js.contains("false"), "Should have pure: false");
    }

    #[test]
    fn test_generate_non_standalone_pipe_definition() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("LegacyPipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadataBuilder::new(Ident::from("LegacyPipe"), type_expr)
            .pipe_name(Ident::from("legacy"))
            .pure(true)
            .is_standalone(false)
            .build();

        let definition = generate_pipe_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&definition.pipe_definition);

        // Should have standalone: false when not standalone
        assert!(js.contains("standalone"), "Should contain standalone property");
        assert!(js.contains("false"), "Should have standalone: false");
    }

    #[test]
    fn test_pipe_definition_is_pure() {
        // Test that the generated expression has pure=true set on the function call.
        // This enables tree-shaking via the @__PURE__ annotation.
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TreeShakablePipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadataBuilder::new(Ident::from("TreeShakablePipe"), type_expr)
            .pipe_name(Ident::from("treeshakable"))
            .pure(true)
            .is_standalone(true)
            .build();

        let definition = generate_pipe_definition(&allocator, &metadata);

        // The pipe_definition should be an InvokeFunction with pure=true
        match &definition.pipe_definition {
            OutputExpression::InvokeFunction(invoke) => {
                assert!(invoke.pure, "Pipe definition should be marked as pure");
            }
            _ => panic!("Expected InvokeFunction expression"),
        }
    }
}

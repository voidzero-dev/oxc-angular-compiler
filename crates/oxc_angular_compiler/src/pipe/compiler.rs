//! Pipe compilation implementation.
//!
//! Ported from Angular's `render3/r3_pipe_compiler.ts`.

use oxc_allocator::{Allocator, Box, Vec};
use oxc_str::Ident;

use super::metadata::R3PipeMetadata;
use crate::output::ast::{
    InvokeFunctionExpr, LiteralExpr, LiteralMapEntry, LiteralMapExpr, LiteralValue,
    OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Result of compiling a pipe.
#[derive(Debug)]
pub struct PipeCompileResult<'a> {
    /// The compiled expression: `ɵɵdefinePipe({...})`
    pub expression: OutputExpression<'a>,

    /// Additional statements (empty for pipes).
    pub statements: Vec<'a, OutputStatement<'a>>,
}

/// Compiles a pipe from its metadata.
///
/// This is the main entry point for pipe compilation.
/// Generates code like:
/// ```javascript
/// ɵpipe = ɵɵdefinePipe({
///   name: "pipeName",
///   type: PipeClass,
///   pure: true,
///   standalone: true
/// })
/// ```
pub fn compile_pipe<'a>(
    allocator: &'a Allocator,
    metadata: &R3PipeMetadata<'a>,
) -> PipeCompileResult<'a> {
    compile_pipe_from_metadata(allocator, metadata)
}

/// Internal implementation of pipe compilation.
pub fn compile_pipe_from_metadata<'a>(
    allocator: &'a Allocator,
    metadata: &R3PipeMetadata<'a>,
) -> PipeCompileResult<'a> {
    // Build the definition map
    let definition_map = build_definition_map(allocator, metadata);

    // Create the expression: ɵɵdefinePipe(definitionMap)
    let expression = create_define_pipe_call(allocator, definition_map);

    PipeCompileResult { expression, statements: Vec::new_in(allocator) }
}

/// Builds the definition map for the pipe.
fn build_definition_map<'a>(
    allocator: &'a Allocator,
    metadata: &R3PipeMetadata<'a>,
) -> Vec<'a, LiteralMapEntry<'a>> {
    let mut entries = Vec::new_in(allocator);

    // name: literal(metadata.pipeName ?? metadata.name)
    let pipe_name = metadata.pipe_name.clone().unwrap_or_else(|| metadata.name.clone());
    entries.push(LiteralMapEntry {
        key: Ident::from("name"),
        value: OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(pipe_name), source_span: None },
            allocator,
        )),
        quoted: false,
    });

    // type: metadata.type.value
    entries.push(LiteralMapEntry {
        key: Ident::from("type"),
        value: metadata.r#type.clone_in(allocator),
        quoted: false,
    });

    // pure: literal(metadata.pure)
    entries.push(LiteralMapEntry {
        key: Ident::from("pure"),
        value: OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(metadata.pure), source_span: None },
            allocator,
        )),
        quoted: false,
    });

    // standalone: only included if false (Angular's runtime defaults standalone to true)
    if !metadata.is_standalone {
        entries.push(LiteralMapEntry {
            key: Ident::from("standalone"),
            value: OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Boolean(false), source_span: None },
                allocator,
            )),
            quoted: false,
        });
    }

    entries
}

/// Creates the `ɵɵdefinePipe({...})` call expression.
fn create_define_pipe_call<'a>(
    allocator: &'a Allocator,
    definition_map: Vec<'a, LiteralMapEntry<'a>>,
) -> OutputExpression<'a> {
    // Create i0.ɵɵdefinePipe
    let define_pipe_fn = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(Identifiers::DEFINE_PIPE),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create the literal map expression
    let map_expr = OutputExpression::LiteralMap(Box::new_in(
        LiteralMapExpr { entries: definition_map, source_span: None },
        allocator,
    ));

    // Create the function call
    let mut args = Vec::new_in(allocator);
    args.push(map_expr);

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(define_pipe_fn, allocator),
            args,
            pure: true, // definePipe is a pure function
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_compile_pure_pipe() {
        let allocator = Allocator::default();
        let name = Ident::from("TestPipe");
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestPipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadata {
            name: name.clone(),
            pipe_name: Some(Ident::from("test")),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            pure: true,
            is_standalone: false,
        };

        let result = compile_pipe(&allocator, &metadata);

        // Emit to string to verify output
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);
        println!("Emitted output: {}", output);
        assert!(output.contains("definePipe"));
        assert!(output.contains("test")); // pipe name (without quotes as may be emitted differently)
        assert!(output.contains("pure"));
    }

    #[test]
    fn test_compile_impure_pipe() {
        let allocator = Allocator::default();
        let name = Ident::from("ImpurePipe");
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ImpurePipe"), source_span: None },
            &allocator,
        ));

        let metadata = R3PipeMetadata {
            name: name.clone(),
            pipe_name: None, // Will use class name
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            pure: false,
            is_standalone: false,
        };

        let result = compile_pipe(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("ImpurePipe")); // Uses class name when pipe_name is None
        assert!(output.contains("false")); // pure: false
    }

    #[test]
    fn test_compile_standalone_pipe() {
        let allocator = Allocator::default();
        let name = Ident::from("StandalonePipe");
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("StandalonePipe"), source_span: None },
            &allocator,
        ));

        // Standalone = true means DON'T include standalone in output (it's the default)
        let metadata = R3PipeMetadata {
            name: name.clone(),
            pipe_name: Some(Ident::from("standalone")),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            pure: true,
            is_standalone: true,
        };

        let result = compile_pipe(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // When is_standalone is true, "standalone" should NOT appear in output (default is true)
        assert!(!output.contains("standalone:"));
    }

    #[test]
    fn test_compile_non_standalone_pipe() {
        let allocator = Allocator::default();
        let name = Ident::from("NonStandalonePipe");
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("NonStandalonePipe"), source_span: None },
            &allocator,
        ));

        // Non-standalone pipes should include standalone: false
        let metadata = R3PipeMetadata {
            name: name.clone(),
            pipe_name: Some(Ident::from("legacy")),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            pure: true,
            is_standalone: false,
        };

        let result = compile_pipe(&allocator, &metadata);
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // When is_standalone is false, "standalone: false" should appear
        assert!(output.contains("standalone"));
        assert!(output.contains("false"));
    }
}

//! Directive definition generation (ɵdir and ɵfac).
//!
//! This module generates the Angular runtime definitions that are added
//! as static properties on directive classes:
//!
//! - `ɵdir`: Directive definition created by `ɵɵdefineDirective()`
//! - `ɵfac`: Factory function for instantiating the directive
//!
//! These definitions are used by Angular's runtime to:
//! - Match the directive to elements via selector
//! - Handle host bindings and listeners
//! - Inject dependencies
//! - Manage directive lifecycle

use oxc_allocator::{Allocator, Vec};

use super::compiler::compile_directive;
use super::metadata::R3DirectiveMetadata;
use crate::factory::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DependencyMetadata, R3FactoryDeps,
    R3FactoryMetadata, compile_factory_function,
};
use crate::output::ast::OutputExpression;

/// Result of generating directive definitions.
pub struct DirectiveDefinitions<'a> {
    /// The ɵdir definition (directive metadata for Angular runtime).
    pub dir_definition: OutputExpression<'a>,

    /// The ɵfac factory function.
    pub fac_definition: OutputExpression<'a>,

    /// The next available pool index after compilation.
    /// Used to track constant pool usage across multiple directives in the same file.
    pub next_pool_index: u32,
}

/// Generate ɵdir and ɵfac definitions for a directive.
///
/// # Arguments
///
/// * `allocator` - Memory allocator
/// * `metadata` - Directive metadata extracted from decorator
/// * `pool_starting_index` - Starting index for the constant pool to avoid conflicts
///
/// # Returns
///
/// The ɵdir and ɵfac definitions as output expressions, along with the next pool index.
///
/// # Example Output
///
/// ```javascript
/// // ɵdir definition:
/// MyDirective.ɵdir = /*@__PURE__*/ i0.ɵɵdefineDirective({
///   type: MyDirective,
///   selectors: [["", "myDir", ""]],
///   inputs: { prop: "prop" },
///   outputs: { click: "click" },
///   hostBindings: function(rf, ctx) { ... },
///   features: [i0.ɵɵNgOnChangesFeature]
/// });
///
/// // ɵfac definition:
/// MyDirective.ɵfac = function MyDirective_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || MyDirective)(
///     i0.ɵɵdirectiveInject(ServiceA),
///     i0.ɵɵdirectiveInject(ServiceB, 8)  // 8 = Optional flag
///   );
/// };
/// ```
pub fn generate_directive_definitions<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    pool_starting_index: u32,
) -> DirectiveDefinitions<'a> {
    // IMPORTANT: Generate ɵfac BEFORE ɵdir to match Angular's namespace index assignment order.
    // Angular processes results in order [fac, def, ...] during the transform phase
    // (see packages/compiler-cli/src/ngtsc/annotations/directive/src/handler.ts:461-468),
    // so factory dependencies get registered first, followed by directive definition dependencies.
    // This ensures namespace indices (i0, i1, i2, ...) are assigned in the same order.
    let fac_definition = generate_fac_definition(allocator, metadata);
    let (dir_definition, next_pool_index) =
        generate_dir_definition(allocator, metadata, pool_starting_index);

    DirectiveDefinitions { dir_definition, fac_definition, next_pool_index }
}

/// Generate the ɵdir definition.
///
/// This calls `compile_directive()` which creates an expression like:
/// ```javascript
/// /*@__PURE__*/ i0.ɵɵdefineDirective({
///   type: MyDirective,
///   selectors: [["", "myDir", ""]],
///   inputs: { prop: "prop" },
///   outputs: { click: "click" },
///   hostBindings: function(rf, ctx) { ... },
///   features: [i0.ɵɵNgOnChangesFeature]
/// })
/// ```
///
/// Returns a tuple of (expression, next_pool_index).
fn generate_dir_definition<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
    pool_starting_index: u32,
) -> (OutputExpression<'a>, u32) {
    let result = compile_directive(allocator, metadata, pool_starting_index);
    (result.expression, result.next_pool_index)
}

/// Generate the ɵfac factory function.
///
/// Creates an expression like:
/// ```javascript
/// // Without dependencies:
/// function DirectiveClass_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || DirectiveClass)();
/// }
///
/// // With dependencies:
/// function DirectiveClass_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || DirectiveClass)(
///     i0.ɵɵdirectiveInject(ServiceA),
///     i0.ɵɵdirectiveInject(ServiceB, 8)  // 8 = Optional flag
///   );
/// }
/// ```
///
/// Ported from: `packages/compiler/src/render3/r3_factory.ts`
fn generate_fac_definition<'a>(
    allocator: &'a Allocator,
    metadata: &R3DirectiveMetadata<'a>,
) -> OutputExpression<'a> {
    // Factory function name: DirectiveName_Factory
    let factory_name = allocator.alloc_str(&format!("{}_Factory", metadata.name));

    // Convert deps to R3FactoryDeps
    // R3FactoryDeps::None means "no constructor, use inherited factory"
    // R3FactoryDeps::Valid(empty vec) means "constructor exists but has no deps"
    // We should only use None when uses_inheritance is true AND deps is None
    let factory_deps = match &metadata.deps {
        Some(deps) => {
            // Clone deps into a new Vec for R3FactoryDeps
            let mut factory_deps: Vec<'a, R3DependencyMetadata<'a>> =
                Vec::with_capacity_in(deps.len(), allocator);
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
        None => {
            // Only use inherited factory pattern if the class actually extends another class
            // If uses_inheritance is false, the class has a no-arg constructor (or implicit one)
            if metadata.uses_inheritance {
                R3FactoryDeps::None
            } else {
                // Empty deps - constructor with no parameters
                R3FactoryDeps::Valid(Vec::new_in(allocator))
            }
        }
    };

    // Create factory metadata
    let factory_meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
        name: metadata.name,
        type_expr: metadata.r#type.clone_in(allocator),
        type_decl: metadata.r#type.clone_in(allocator),
        type_argument_count: metadata.type_argument_count,
        deps: factory_deps,
        target: FactoryTarget::Directive,
    });

    // Compile the factory function
    let result = compile_factory_function(allocator, &factory_meta, factory_name);
    result.expression
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::directive::metadata::R3HostMetadata;
    use crate::output::ast::ReadVarExpr;
    use crate::output::emitter::JsEmitter;
    use oxc_allocator::Box;
    use oxc_span::Ident;

    fn create_test_metadata<'a>(allocator: &'a Allocator) -> R3DirectiveMetadata<'a> {
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestDirective"), source_span: None },
            allocator,
        ));

        R3DirectiveMetadata {
            name: Ident::from("TestDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None,
            selector: Some(Ident::from("[testDir]")),
            queries: Vec::new_in(allocator),
            view_queries: Vec::new_in(allocator),
            host: R3HostMetadata::new(allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(allocator),
            outputs: Vec::new_in(allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(allocator),
        }
    }

    #[test]
    fn test_generate_directive_definitions() {
        let allocator = Allocator::default();
        let metadata = create_test_metadata(&allocator);

        let definitions = generate_directive_definitions(&allocator, &metadata, 0);

        let emitter = JsEmitter::new();

        // Check dir definition
        let dir_js = emitter.emit_expression(&definitions.dir_definition);
        assert!(dir_js.contains("defineDirective"));
        assert!(dir_js.contains("TestDirective"));
        assert!(dir_js.contains("selectors"));

        // Check fac definition
        let fac_js = emitter.emit_expression(&definitions.fac_definition);
        assert!(fac_js.contains("TestDirective_Factory"));
        assert!(fac_js.contains("__ngFactoryType__"));
    }

    #[test]
    fn test_generate_fac_definition_without_deps_no_inheritance() {
        // When deps is None but uses_inheritance is false, we should generate
        // a simple factory (class has implicit/no-arg constructor, not inherited)
        let allocator = Allocator::default();
        let metadata = create_test_metadata(&allocator);

        let fac = generate_fac_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&fac);

        // Should have simple factory (no inherited factory pattern)
        // because uses_inheritance is false
        assert!(js.contains("TestDirective_Factory"));
        assert!(js.contains("__ngFactoryType__"));
        assert!(js.contains("new"));
        // Should NOT have getInheritedFactory
        assert!(!js.contains("getInheritedFactory"));
    }

    #[test]
    fn test_generate_fac_definition_with_inheritance() {
        // When deps is None AND uses_inheritance is true, we should generate
        // an inherited factory pattern
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ChildDirective"), source_span: None },
            &allocator,
        ));

        let metadata = R3DirectiveMetadata {
            name: Ident::from("ChildDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: None, // No explicit constructor deps
            selector: Some(Ident::from("[childDir]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: true, // Key: extends a base class
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let fac = generate_fac_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&fac);

        // Should have inherited factory pattern (IIFE with getInheritedFactory)
        // because uses_inheritance is true
        assert!(js.contains("getInheritedFactory") || js.contains("ɵɵgetInheritedFactory"));
        assert!(js.contains("ChildDirective"));
    }

    #[test]
    fn test_generate_fac_definition_with_empty_deps() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestDirective"), source_span: None },
            &allocator,
        ));

        let metadata = R3DirectiveMetadata {
            name: Ident::from("TestDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: Some(Vec::new_in(&allocator)), // Empty deps - has constructor but no params
            selector: Some(Ident::from("[testDir]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let fac = generate_fac_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&fac);

        // Should have simple factory (no inherited factory pattern)
        assert!(js.contains("TestDirective_Factory"));
        assert!(js.contains("__ngFactoryType__"));
        assert!(js.contains("new"));
        // Should NOT have getInheritedFactory
        assert!(!js.contains("getInheritedFactory"));
    }

    #[test]
    fn test_generate_fac_definition_with_deps() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestDirective"), source_span: None },
            &allocator,
        ));

        let dep_token = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("SomeService"), source_span: None },
            &allocator,
        ));

        let mut deps = Vec::new_in(&allocator);
        deps.push(crate::factory::R3DependencyMetadata::simple(dep_token));

        let metadata = R3DirectiveMetadata {
            name: Ident::from("TestDirective"),
            r#type: type_expr,
            type_argument_count: 0,
            deps: Some(deps),
            selector: Some(Ident::from("[testDir]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: R3HostMetadata::new(&allocator),
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        let fac = generate_fac_definition(&allocator, &metadata);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&fac);

        // Should have directiveInject call (not just inject)
        assert!(js.contains("directiveInject"));
        assert!(js.contains("SomeService"));
    }

    #[test]
    fn test_generate_dir_definition() {
        let allocator = Allocator::default();
        let metadata = create_test_metadata(&allocator);

        let (dir, _next_pool_index) = generate_dir_definition(&allocator, &metadata, 0);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&dir);

        assert!(js.contains("ɵɵdefineDirective"));
        assert!(js.contains("type"));
        assert!(js.contains("TestDirective"));
        assert!(js.contains("selectors"));
        assert!(js.contains("testDir"));
    }

    #[test]
    fn test_pool_index_tracking_for_multiple_directives() {
        // Test that multiple directives with host bindings use unique pool indices
        use crate::directive::metadata::R3HostMetadata;

        let allocator = Allocator::default();

        // Create first directive with host bindings
        let type_expr1 = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("Dir1"), source_span: None },
            &allocator,
        ));

        let mut host1 = R3HostMetadata::new(&allocator);
        // Add a property binding which will use the constant pool
        host1.properties.push((Ident::from("[attr.role]"), Ident::from("'button'")));

        let metadata1 = R3DirectiveMetadata {
            name: Ident::from("Dir1"),
            r#type: type_expr1,
            type_argument_count: 0,
            deps: None,
            selector: Some(Ident::from("[dir1]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: host1,
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        // Compile first directive
        let definitions1 = generate_directive_definitions(&allocator, &metadata1, 0);
        let next_index = definitions1.next_pool_index;

        // The next_pool_index should be 0 when no constants are pooled
        // (static attribute bindings like role='button' don't require pool entries)
        // This test validates that the pool index is tracked and returned correctly

        // Create second directive with host bindings using the returned pool index
        let type_expr2 = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("Dir2"), source_span: None },
            &allocator,
        ));

        let mut host2 = R3HostMetadata::new(&allocator);
        host2.properties.push((Ident::from("[attr.id]"), Ident::from("'test'")));

        let metadata2 = R3DirectiveMetadata {
            name: Ident::from("Dir2"),
            r#type: type_expr2,
            type_argument_count: 0,
            deps: None,
            selector: Some(Ident::from("[dir2]")),
            queries: Vec::new_in(&allocator),
            view_queries: Vec::new_in(&allocator),
            host: host2,
            uses_on_changes: false,
            inputs: Vec::new_in(&allocator),
            outputs: Vec::new_in(&allocator),
            uses_inheritance: false,
            export_as: Vec::new_in(&allocator),
            providers: None,
            is_standalone: true,
            is_signal: false,
            host_directives: Vec::new_in(&allocator),
        };

        // Compile second directive starting from where first left off
        let definitions2 = generate_directive_definitions(&allocator, &metadata2, next_index);

        // Verify both directives compiled successfully
        let emitter = JsEmitter::new();
        let js1 = emitter.emit_expression(&definitions1.dir_definition);
        let js2 = emitter.emit_expression(&definitions2.dir_definition);

        assert!(js1.contains("Dir1"), "First directive should contain Dir1");
        assert!(js2.contains("Dir2"), "Second directive should contain Dir2");

        // The pool indices should be tracked - even if they're the same in this case,
        // the mechanism is in place to avoid conflicts when constants are actually pooled
        assert!(
            definitions2.next_pool_index >= next_index,
            "Pool index should advance or stay same, got {} from {}",
            definitions2.next_pool_index,
            next_index
        );
    }
}

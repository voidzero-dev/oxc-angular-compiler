//! Dependency injection types and factory compilation.
//!
//! This module provides:
//! - `R3DependencyMetadata`: Metadata for a single constructor parameter
//! - `InjectFlags`: Flags for dependency injection decorators
//! - `FactoryTarget`: Target type for factory generation
//! - Factory compilation functions for DI-aware factory generation
//!
//! Ported from: `packages/compiler/src/render3/r3_factory.ts`

use oxc_allocator::{Allocator, Box, Vec as OxcVec};
use oxc_span::Ident;

use super::namespace_registry::NamespaceRegistry;
use crate::output::ast::{
    InvokeFunctionExpr, LiteralExpr, LiteralValue, OutputExpression, ReadPropExpr, ReadVarExpr,
};
use crate::r3::Identifiers;

/// Metadata for a single constructor parameter that needs to be injected.
///
/// Corresponds to `R3DependencyMetadata` in the TypeScript compiler.
/// See: `packages/compiler/src/render3/r3_factory.ts:68-101`
///
/// Note: Unlike TypeScript which stores OutputExpression, we store token names
/// as Atom strings and generate expressions at compile time. This works better
/// with oxc's arena allocation pattern.
#[derive(Debug, Clone)]
pub struct R3DependencyMetadata<'a> {
    /// The injection token name (service class name, InjectionToken, etc.).
    /// `None` represents an invalid/unresolved dependency.
    pub token: Option<Ident<'a>>,

    /// The source module of the token (e.g., "@angular/core", "@angular/router").
    /// Used to generate proper namespace aliases for imported dependencies.
    /// `None` for local dependencies or when the source is unknown.
    pub token_source_module: Option<Ident<'a>>,

    /// Whether the token has an existing named import in the source file.
    /// If true, the token can be used with a bare name instead of namespace prefix.
    /// This mimics Angular's import reuse behavior where existing named imports
    /// are reused rather than generating new namespace imports.
    ///
    /// Example: `import { DIALOG_DATA } from "@angular/cdk/dialog"` allows using
    /// `DIALOG_DATA` directly instead of `i1.DIALOG_DATA`.
    pub has_named_import: bool,

    /// For `@Attribute()` dependencies, the attribute name.
    /// `None` for regular dependencies.
    pub attribute_name: Option<Ident<'a>>,

    /// Whether `@Host()` decorator is present.
    pub host: bool,

    /// Whether `@Optional()` decorator is present.
    pub optional: bool,

    /// Whether `@Self()` decorator is present.
    pub self_: bool,

    /// Whether `@SkipSelf()` decorator is present.
    pub skip_self: bool,
}

impl<'a> R3DependencyMetadata<'a> {
    /// Create a new dependency metadata with default flags.
    pub fn new(token: Ident<'a>) -> Self {
        Self {
            token: Some(token),
            token_source_module: None,
            has_named_import: false,
            attribute_name: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        }
    }

    /// Create an invalid dependency (for error handling).
    pub fn invalid() -> Self {
        Self {
            token: None,
            token_source_module: None,
            has_named_import: false,
            attribute_name: None,
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        }
    }

    /// Create a dependency with `@Optional()` decorator.
    pub fn with_optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Create a dependency with `@Host()` decorator.
    pub fn with_host(mut self) -> Self {
        self.host = true;
        self
    }

    /// Create a dependency with `@Self()` decorator.
    pub fn with_self(mut self) -> Self {
        self.self_ = true;
        self
    }

    /// Create a dependency with `@SkipSelf()` decorator.
    pub fn with_skip_self(mut self) -> Self {
        self.skip_self = true;
        self
    }

    /// Create an `@Attribute()` dependency.
    pub fn attribute(attribute_name: Ident<'a>) -> Self {
        Self {
            token: Some(attribute_name.clone()),
            token_source_module: None,
            has_named_import: false,
            attribute_name: Some(attribute_name),
            host: false,
            optional: false,
            self_: false,
            skip_self: false,
        }
    }

    /// Set that this token has an existing named import.
    pub fn with_named_import(mut self) -> Self {
        self.has_named_import = true;
        self
    }

    /// Set the source module for the token.
    pub fn with_token_source_module(mut self, source_module: Ident<'a>) -> Self {
        self.token_source_module = Some(source_module);
        self
    }
}

/// Flags for dependency injection configuration.
///
/// These flags modify how Angular's injector resolves dependencies.
/// Corresponds to `InjectFlags` in Angular core.
/// See: `packages/core/src/di/interface/injector.ts`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct InjectFlags(u8);

impl InjectFlags {
    /// Default injection behavior.
    pub const DEFAULT: Self = Self(0);
    /// Look for the dependency in the host element's injector.
    pub const HOST: Self = Self(1 << 0);
    /// Only look for the dependency in the local injector.
    pub const SELF: Self = Self(1 << 1);
    /// Skip the local injector and look in parent injectors.
    pub const SKIP_SELF: Self = Self(1 << 2);
    /// Return `null` if the dependency is not found.
    pub const OPTIONAL: Self = Self(1 << 3);
    /// Used internally for pipes.
    pub const FOR_PIPE: Self = Self(1 << 4);

    /// Check if no flags are set (default behavior).
    pub fn is_default(self) -> bool {
        self.0 == 0
    }

    /// Get the numeric value for code generation.
    pub fn value(self) -> u8 {
        self.0
    }

    /// Combine flags using bitwise OR.
    pub fn or(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOr for InjectFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// The target type for factory generation.
///
/// Determines which inject function to use:
/// - Components/Directives/Pipes: `ɵɵdirectiveInject`
/// - Injectables/NgModules: `ɵɵinject`
///
/// See: `packages/compiler/src/compiler_facade_interface.ts:120-126`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FactoryTarget {
    /// A directive class.
    Directive = 0,
    /// A component class.
    #[default]
    Component = 1,
    /// An injectable service.
    Injectable = 2,
    /// A pipe class.
    Pipe = 3,
    /// An NgModule class.
    NgModule = 4,
}

/// Get the inject function name for a given target.
///
/// Components, Directives, and Pipes use `ɵɵdirectiveInject`.
/// Injectables and NgModules use `ɵɵinject`.
///
/// See: `packages/compiler/src/render3/r3_factory.ts:313-324`
pub fn get_inject_fn_name(target: FactoryTarget) -> &'static str {
    match target {
        FactoryTarget::Component | FactoryTarget::Directive | FactoryTarget::Pipe => {
            Identifiers::DIRECTIVE_INJECT
        }
        FactoryTarget::Injectable | FactoryTarget::NgModule => Identifiers::INJECT,
    }
}

/// Compile the injection expressions for a list of dependencies.
///
/// Returns the arguments to pass to the constructor.
///
/// See: `packages/compiler/src/render3/r3_factory.ts:208-265`
pub fn compile_inject_dependencies<'a>(
    allocator: &'a Allocator,
    deps: &[R3DependencyMetadata<'a>],
    target: FactoryTarget,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OxcVec<'a, OutputExpression<'a>> {
    let mut args = OxcVec::with_capacity_in(deps.len(), allocator);

    for (index, dep) in deps.iter().enumerate() {
        args.push(compile_inject_dependency(allocator, dep, target, index, namespace_registry));
    }

    args
}

/// Compile a single dependency injection expression.
///
/// Generates one of:
/// - `ɵɵinvalidFactoryDep(index)` - for invalid dependencies
/// - `ɵɵinjectAttribute(attrName)` - for `@Attribute()` dependencies
/// - `ɵɵdirectiveInject(token)` - for component/directive dependencies (local)
/// - `ɵɵdirectiveInject(i1.Token)` - for imported dependencies with namespace
/// - `ɵɵdirectiveInject(token, flags)` - with optional/host/self/skipSelf flags
/// - `ɵɵinject(token)` - for injectable/module dependencies
/// - `ɵɵinject(token, flags)` - with flags
///
/// See: `packages/compiler/src/render3/r3_factory.ts:217-258`
fn compile_inject_dependency<'a>(
    allocator: &'a Allocator,
    dep: &R3DependencyMetadata<'a>,
    target: FactoryTarget,
    index: usize,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    // Case 1: Invalid dependency - token is None
    let Some(ref token_name) = dep.token else {
        return create_invalid_factory_dep_call(allocator, index);
    };

    // Case 2: @Attribute() dependency
    if let Some(ref attr_name) = dep.attribute_name {
        return create_inject_attribute_call(allocator, attr_name.clone());
    }

    // Case 3: Regular token injection
    // Build flags from decorators
    let mut flags = InjectFlags::DEFAULT;
    if dep.self_ {
        flags = flags | InjectFlags::SELF;
    }
    if dep.skip_self {
        flags = flags | InjectFlags::SKIP_SELF;
    }
    if dep.host {
        flags = flags | InjectFlags::HOST;
    }
    if dep.optional {
        flags = flags | InjectFlags::OPTIONAL;
    }
    if target == FactoryTarget::Pipe {
        flags = flags | InjectFlags::FOR_PIPE;
    }

    // Get the appropriate inject function
    let inject_fn_name = get_inject_fn_name(target);

    // Build the token expression based on whether it's an imported dependency
    let token_expr = create_token_expression(allocator, dep, token_name, namespace_registry);

    // Build the call expression
    // Only include flags if they are non-default OR if optional is true
    // (Angular special-cases optional to always include the flags param)
    if flags.is_default() && !dep.optional {
        create_inject_call_with_expr(allocator, inject_fn_name, token_expr, None)
    } else {
        create_inject_call_with_expr(allocator, inject_fn_name, token_expr, Some(flags.value()))
    }
}

/// Create a token expression, either as a bare variable or with namespace prefix.
///
/// For imported dependencies:
/// - If `has_named_import` is true, uses bare `TokenName` (reuses existing import)
/// - Otherwise, generates `i1.AuthService` (namespace import)
///
/// For local dependencies, generates just `AuthService`.
///
/// This mimics Angular's import reuse behavior where existing named imports
/// are reused rather than generating new namespace imports.
fn create_token_expression<'a>(
    allocator: &'a Allocator,
    dep: &R3DependencyMetadata<'a>,
    token_name: &Ident<'a>,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OutputExpression<'a> {
    if let Some(ref source_module) = dep.token_source_module {
        // Check if this token has an existing named import that can be reused
        if dep.has_named_import {
            // Reuse existing named import - use bare token name
            // This matches Angular's behavior: import { DIALOG_DATA } from "@angular/cdk/dialog"
            // allows using `DIALOG_DATA` directly instead of `i1.DIALOG_DATA`
            return OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: token_name.clone(), source_span: None },
                allocator,
            ));
        }

        // Imported dependency without named import - use namespace.TokenName (e.g., i1.AuthService)
        let namespace = namespace_registry.get_or_assign(source_module);
        OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: namespace, source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: token_name.clone(),
                optional: false,
                source_span: None,
            },
            allocator,
        ))
    } else {
        // Local dependency - use bare token name
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: token_name.clone(), source_span: None },
            allocator,
        ))
    }
}

/// Create an `i0.ɵɵinvalidFactoryDep(index)` call.
fn create_invalid_factory_dep_call<'a>(
    allocator: &'a Allocator,
    index: usize,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, Identifiers::INVALID_FACTORY_DEP);

    let mut args = OxcVec::with_capacity_in(1, allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(index as f64), source_span: None },
        allocator,
    )));

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create an `i0.ɵɵinjectAttribute(attrName)` call.
fn create_inject_attribute_call<'a>(
    allocator: &'a Allocator,
    attr_name: Ident<'a>,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, Identifiers::INJECT_ATTRIBUTE);

    let mut args = OxcVec::with_capacity_in(1, allocator);
    // Attribute name is passed as a string literal
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::String(attr_name), source_span: None },
        allocator,
    )));

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create an inject call with a pre-built token expression.
///
/// Generates: `i0.ɵɵdirectiveInject(tokenExpr)` or `i0.ɵɵinject(tokenExpr, flags)`.
/// The token expression can be either a bare variable or a namespaced reference.
fn create_inject_call_with_expr<'a>(
    allocator: &'a Allocator,
    fn_name: &'static str,
    token_expr: OutputExpression<'a>,
    flags: Option<u8>,
) -> OutputExpression<'a> {
    let fn_expr = create_angular_fn_ref(allocator, fn_name);

    let capacity = if flags.is_some() { 2 } else { 1 };
    let mut args = OxcVec::with_capacity_in(capacity, allocator);

    // Token expression (may be a variable reference or namespaced property access)
    args.push(token_expr);

    if let Some(flags_value) = flags {
        args.push(OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(flags_value as f64), source_span: None },
            allocator,
        )));
    }

    OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(fn_expr, allocator),
            args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Create an `i0.functionName` reference expression.
fn create_angular_fn_ref<'a>(
    allocator: &'a Allocator,
    fn_name: &'static str,
) -> OutputExpression<'a> {
    OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(fn_name),
            optional: false,
            source_span: None,
        },
        allocator,
    ))
}

/// Generate a DI-aware factory function body.
///
/// Returns the expressions needed for the constructor arguments.
/// The caller should use these as arguments to the `new` expression.
pub fn generate_factory_di_args<'a>(
    allocator: &'a Allocator,
    deps: &[R3DependencyMetadata<'a>],
    target: FactoryTarget,
    namespace_registry: &mut NamespaceRegistry<'a>,
) -> OxcVec<'a, OutputExpression<'a>> {
    compile_inject_dependencies(allocator, deps, target, namespace_registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_simple_dependency() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("MyService"));
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵdirectiveInject"));
        assert!(js.contains("MyService"));
        assert!(!js.contains(",")); // No flags argument
    }

    #[test]
    fn test_optional_dependency() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("MyService")).with_optional();
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵdirectiveInject"));
        assert!(js.contains("MyService"));
        assert!(js.contains("8")); // InjectFlags.OPTIONAL = 8
    }

    #[test]
    fn test_host_dependency() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("HostService")).with_host();
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵdirectiveInject"));
        assert!(js.contains("HostService"));
        assert!(js.contains("1")); // InjectFlags.HOST = 1
    }

    #[test]
    fn test_combined_flags() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("Service")).with_optional().with_host();
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵdirectiveInject"));
        assert!(js.contains("9")); // OPTIONAL (8) | HOST (1) = 9
    }

    #[test]
    fn test_injectable_uses_inject() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("Service"));
        let mut registry = NamespaceRegistry::new(&allocator);

        let result = compile_inject_dependency(
            &allocator,
            &dep,
            FactoryTarget::Injectable,
            0,
            &mut registry,
        );

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵinject")); // Not ɵɵdirectiveInject
        assert!(!js.contains("ɵɵdirectiveInject"));
    }

    #[test]
    fn test_invalid_dependency() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::invalid();
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 2, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵinvalidFactoryDep"));
        assert!(js.contains("2")); // index
    }

    #[test]
    fn test_attribute_dependency() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::attribute(Ident::from("title"));
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        assert!(js.contains("ɵɵinjectAttribute"));
        assert!(js.contains("title"));
    }

    #[test]
    fn test_imported_dependency_with_namespace() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("AuthService"))
            .with_token_source_module(Ident::from("@app/auth"));
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // Should generate i0.ɵɵdirectiveInject(i1.AuthService)
        assert!(js.contains("ɵɵdirectiveInject"));
        assert!(js.contains("i1.AuthService"));
    }

    #[test]
    fn test_multiple_imported_dependencies_same_module() {
        let allocator = Allocator::default();
        let dep1 = R3DependencyMetadata::new(Ident::from("AuthService"))
            .with_token_source_module(Ident::from("@app/auth"));
        let dep2 = R3DependencyMetadata::new(Ident::from("UserService"))
            .with_token_source_module(Ident::from("@app/auth"));
        let deps = vec![dep1, dep2];
        let mut registry = NamespaceRegistry::new(&allocator);

        let results =
            compile_inject_dependencies(&allocator, &deps, FactoryTarget::Component, &mut registry);

        let emitter = JsEmitter::new();
        let js1 = emitter.emit_expression(&results[0]);
        let js2 = emitter.emit_expression(&results[1]);

        // Both should use the same namespace (i1)
        assert!(js1.contains("i1.AuthService"));
        assert!(js2.contains("i1.UserService"));
    }

    #[test]
    fn test_multiple_imported_dependencies_different_modules() {
        let allocator = Allocator::default();
        let dep1 = R3DependencyMetadata::new(Ident::from("AuthService"))
            .with_token_source_module(Ident::from("@app/auth"));
        let dep2 = R3DependencyMetadata::new(Ident::from("HttpService"))
            .with_token_source_module(Ident::from("@app/http"));
        let deps = vec![dep1, dep2];
        let mut registry = NamespaceRegistry::new(&allocator);

        let results =
            compile_inject_dependencies(&allocator, &deps, FactoryTarget::Component, &mut registry);

        let emitter = JsEmitter::new();
        let js1 = emitter.emit_expression(&results[0]);
        let js2 = emitter.emit_expression(&results[1]);

        // Should use different namespaces (i1, i2)
        assert!(js1.contains("i1.AuthService"));
        assert!(js2.contains("i2.HttpService"));
    }

    #[test]
    fn test_angular_core_dependency() {
        let allocator = Allocator::default();
        let dep = R3DependencyMetadata::new(Ident::from("ChangeDetectorRef"))
            .with_token_source_module(Ident::from("@angular/core"));
        let mut registry = NamespaceRegistry::new(&allocator);

        let result =
            compile_inject_dependency(&allocator, &dep, FactoryTarget::Component, 0, &mut registry);

        let emitter = JsEmitter::new();
        let js = emitter.emit_expression(&result);

        // @angular/core should always be i0
        assert!(js.contains("i0.ChangeDetectorRef"));
    }
}

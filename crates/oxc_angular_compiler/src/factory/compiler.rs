//! Factory function compilation.
//!
//! Ported from Angular's `render3/r3_factory.ts`.
//!
//! Generates factory functions like:
//! ```javascript
//! function MyClass_Factory(__ngFactoryType__) {
//!   return new (__ngFactoryType__ || MyClass)(ɵɵinject(Dep1), ɵɵinject(Dep2));
//! }
//! ```
//!
//! For inherited factories (no constructor), generates:
//! ```javascript
//! /*@__PURE__*/ (() => {
//!   let ɵMyClass_BaseFactory;
//!   return function MyClass_Factory(__ngFactoryType__) {
//!     return (ɵMyClass_BaseFactory || (ɵMyClass_BaseFactory = ɵɵgetInheritedFactory(MyClass)))(__ngFactoryType__ || MyClass);
//!   };
//! })()
//! ```
//!
//! For delegated factories (useFactory/useClass with deps), generates:
//! ```javascript
//! function MyService_Factory(__ngFactoryType__) {
//!   let __ngConditionalFactory__ = null;
//!   if (__ngFactoryType__) {
//!     __ngConditionalFactory__ = new (__ngFactoryType__ || MyService)();
//!   } else {
//!     __ngConditionalFactory__ = delegatedFactory(ɵɵinject(Dep1), ɵɵinject(Dep2));
//!   }
//!   return __ngConditionalFactory__;
//! }
//! ```

use oxc_allocator::{Allocator, Box, FromIn, Vec};
use oxc_span::Ident;

use super::metadata::{
    FactoryTarget, R3DependencyMetadata, R3FactoryDelegateType, R3FactoryDeps, R3FactoryMetadata,
};
use crate::output::ast::{
    ArrowFunctionBody, ArrowFunctionExpr, BinaryOperator, BinaryOperatorExpr, DeclareVarStmt,
    ExpressionStatement, FnParam, FunctionExpr, IfStmt, InstantiateExpr, InvokeFunctionExpr,
    LiteralExpr, LiteralValue, OutputExpression, OutputStatement, ReadPropExpr, ReadVarExpr,
    ReturnStatement, StmtModifier,
};
use crate::r3::Identifiers;

/// Inject flags matching Angular's InjectFlags enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectFlags(pub u32);

impl InjectFlags {
    /// No special injection flags.
    pub const DEFAULT: u32 = 0;
    /// Inject from host only.
    pub const HOST: u32 = 1;
    /// Inject from self only.
    pub const SELF: u32 = 2;
    /// Skip self when injecting.
    pub const SKIP_SELF: u32 = 4;
    /// Dependency is optional.
    pub const OPTIONAL: u32 = 8;
    /// Injection is for a pipe.
    pub const FOR_PIPE: u32 = 16;
}

/// Result of compiling a factory function.
#[derive(Debug)]
pub struct FactoryCompileResult<'a> {
    /// The compiled factory expression.
    pub expression: OutputExpression<'a>,

    /// Additional statements (usually empty).
    pub statements: Vec<'a, OutputStatement<'a>>,
}

/// Compiles a factory function from metadata.
///
/// Generates code like:
/// ```javascript
/// function MyClass_Factory(__ngFactoryType__) {
///   return new (__ngFactoryType__ || MyClass)(ɵɵdirectiveInject(Dep1));
/// }
/// ```
///
/// For delegated factories (useFactory/useClass with deps), generates the conditional pattern:
/// ```javascript
/// function MyService_Factory(__ngFactoryType__) {
///   let __ngConditionalFactory__ = null;
///   if (__ngFactoryType__) {
///     __ngConditionalFactory__ = new (__ngFactoryType__ || MyService)();
///   } else {
///     __ngConditionalFactory__ = delegatedFactory(ɵɵinject(Dep1), ɵɵinject(Dep2));
///   }
///   return __ngConditionalFactory__;
/// }
/// ```
///
/// See: `packages/compiler/src/render3/r3_factory.ts:106-200`
pub fn compile_factory_function<'a>(
    allocator: &'a Allocator,
    meta: &R3FactoryMetadata<'a>,
    factory_name: &'a str,
) -> FactoryCompileResult<'a> {
    let base = meta.base();
    let factory_type_param = Ident::from("__ngFactoryType__");

    // The type to instantiate via constructor invocation. If there is no delegated factory,
    // meaning this type is always created by constructor invocation, then this is the
    // type-to-create parameter provided by the user (t) if specified, or the current type if not.
    // If there is a delegated factory (which is used to create the current type) then this is
    // only the type-to-create parameter (t).
    // See: r3_factory.ts:115-118
    let type_for_ctor = if meta.is_delegated() {
        // For delegated factories, only use the type parameter
        OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: factory_type_param.clone(), source_span: None },
            allocator,
        ))
    } else {
        // (__ngFactoryType__ || MyClass)
        OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: BinaryOperator::Or,
                lhs: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: factory_type_param.clone(), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                rhs: Box::new_in(base.type_expr.clone_in(allocator), allocator),
                source_span: None,
            },
            allocator,
        ))
    };

    // Build the constructor expression based on deps
    // See: r3_factory.ts:119-129
    let ctor_expr: Option<OutputExpression<'a>> = match &base.deps {
        R3FactoryDeps::Valid(deps) => {
            // new (type)(ɵɵinject(Dep1), ɵɵinject(Dep2), ...)
            let inject_args = inject_dependencies(allocator, deps.as_slice(), base.target);
            Some(OutputExpression::Instantiate(Box::new_in(
                InstantiateExpr {
                    class_expr: Box::new_in(type_for_ctor.clone_in(allocator), allocator),
                    args: inject_args,
                    source_span: None,
                },
                allocator,
            )))
        }
        R3FactoryDeps::Invalid => None,
        R3FactoryDeps::None => {
            // No constructor - use inherited factory pattern.
            // This is handled specially below for non-delegated cases.
            None
        }
    };

    // Check if we need inherited factory pattern for non-delegated case with no deps
    if !meta.is_delegated() && !meta.is_expression() && matches!(&base.deps, R3FactoryDeps::None) {
        return compile_inherited_factory(allocator, base, factory_name);
    }

    let mut body: Vec<'a, OutputStatement<'a>> = Vec::new_in(allocator);
    let ret_expr: Option<OutputExpression<'a>>;

    // Handle the different metadata types
    // See: r3_factory.ts:145-159
    match meta {
        R3FactoryMetadata::Delegated(delegated_meta) => {
            // This type is created with a delegated factory. If a type parameter is not specified,
            // call the factory instead.
            let delegate_args =
                inject_dependencies(allocator, &delegated_meta.delegate_deps, base.target);

            // Either call `new delegate(...)` or `delegate(...)` depending on delegate_type
            let factory_expr = match delegated_meta.delegate_type {
                R3FactoryDelegateType::Class => {
                    // new delegate(...)
                    OutputExpression::Instantiate(Box::new_in(
                        InstantiateExpr {
                            class_expr: Box::new_in(
                                delegated_meta.delegate.clone_in(allocator),
                                allocator,
                            ),
                            args: delegate_args,
                            source_span: None,
                        },
                        allocator,
                    ))
                }
                R3FactoryDelegateType::Function => {
                    // delegate(...)
                    OutputExpression::InvokeFunction(Box::new_in(
                        InvokeFunctionExpr {
                            fn_expr: Box::new_in(
                                delegated_meta.delegate.clone_in(allocator),
                                allocator,
                            ),
                            args: delegate_args,
                            pure: false,
                            optional: false,
                            source_span: None,
                        },
                        allocator,
                    ))
                }
            };

            ret_expr = Some(make_conditional_factory(
                allocator,
                &mut body,
                &factory_type_param,
                ctor_expr,
                factory_expr,
            ));
        }
        R3FactoryMetadata::Expression(expr_meta) => {
            // useValue or useExisting case
            ret_expr = Some(make_conditional_factory(
                allocator,
                &mut body,
                &factory_type_param,
                ctor_expr,
                expr_meta.expression.clone_in(allocator),
            ));
        }
        R3FactoryMetadata::Constructor(_) => {
            // Standard constructor case - just use ctor_expr directly
            ret_expr = ctor_expr;
        }
    }

    // Generate the return or invalidFactory call
    // See: r3_factory.ts:161-177
    match ret_expr {
        None => {
            // The expression cannot be formed so render an `ɵɵinvalidFactory()` call.
            let invalid_factory_call = create_invalid_factory_call(allocator);
            body.push(OutputStatement::Expression(Box::new_in(
                ExpressionStatement { expr: invalid_factory_call, source_span: None },
                allocator,
            )));
        }
        Some(expr) => {
            // Return the result
            body.push(OutputStatement::Return(Box::new_in(
                ReturnStatement { value: expr, source_span: None },
                allocator,
            )));
        }
    }

    // Create the factory function
    let mut params = Vec::new_in(allocator);
    params.push(FnParam { name: factory_type_param });

    let factory_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: Some(Ident::from(factory_name)),
            params,
            statements: body,
            source_span: None,
        },
        allocator,
    ));

    FactoryCompileResult { expression: factory_fn, statements: Vec::new_in(allocator) }
}

/// Creates the conditional factory pattern for delegated/expression factories.
///
/// Generates:
/// ```javascript
/// let __ngConditionalFactory__ = null;
/// if (__ngFactoryType__) {
///   __ngConditionalFactory__ = <ctor_expr>;  // or invalidFactory() if None
/// } else {
///   __ngConditionalFactory__ = <non_ctor_expr>;
/// }
/// ```
///
/// Returns the `__ngConditionalFactory__` variable reference.
///
/// See: `packages/compiler/src/render3/r3_factory.ts:134-143`
fn make_conditional_factory<'a>(
    allocator: &'a Allocator,
    body: &mut Vec<'a, OutputStatement<'a>>,
    factory_type_param: &Ident<'a>,
    ctor_expr: Option<OutputExpression<'a>>,
    non_ctor_expr: OutputExpression<'a>,
) -> OutputExpression<'a> {
    let conditional_factory_var = Ident::from("__ngConditionalFactory__");

    // let __ngConditionalFactory__ = null;
    body.push(OutputStatement::DeclareVar(Box::new_in(
        DeclareVarStmt {
            name: conditional_factory_var.clone(),
            value: Some(OutputExpression::Literal(Box::new_in(
                LiteralExpr { value: LiteralValue::Null, source_span: None },
                allocator,
            ))),
            modifiers: StmtModifier::NONE,
            leading_comment: None,
            source_span: None,
        },
        allocator,
    )));

    // Create the true case: __ngConditionalFactory__ = <ctor_expr> or invalidFactory()
    let true_stmt = match ctor_expr {
        Some(expr) => {
            // __ngConditionalFactory__ = <ctor_expr>
            let assignment = OutputExpression::BinaryOperator(Box::new_in(
                BinaryOperatorExpr {
                    operator: BinaryOperator::Assign,
                    lhs: Box::new_in(
                        OutputExpression::ReadVar(Box::new_in(
                            ReadVarExpr {
                                name: conditional_factory_var.clone(),
                                source_span: None,
                            },
                            allocator,
                        )),
                        allocator,
                    ),
                    rhs: Box::new_in(expr, allocator),
                    source_span: None,
                },
                allocator,
            ));
            OutputStatement::Expression(Box::new_in(
                ExpressionStatement { expr: assignment, source_span: None },
                allocator,
            ))
        }
        None => {
            // invalidFactory()
            let invalid_factory_call = create_invalid_factory_call(allocator);
            OutputStatement::Expression(Box::new_in(
                ExpressionStatement { expr: invalid_factory_call, source_span: None },
                allocator,
            ))
        }
    };

    // Create the false case: __ngConditionalFactory__ = <non_ctor_expr>
    let false_assignment = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Assign,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: conditional_factory_var.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(non_ctor_expr, allocator),
            source_span: None,
        },
        allocator,
    ));
    let false_stmt = OutputStatement::Expression(Box::new_in(
        ExpressionStatement { expr: false_assignment, source_span: None },
        allocator,
    ));

    // Create the if statement
    let mut true_case = Vec::new_in(allocator);
    true_case.push(true_stmt);
    let mut false_case = Vec::new_in(allocator);
    false_case.push(false_stmt);

    body.push(OutputStatement::If(Box::new_in(
        IfStmt {
            condition: OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name: factory_type_param.clone(), source_span: None },
                allocator,
            )),
            true_case,
            false_case,
            source_span: None,
        },
        allocator,
    )));

    // Return the variable reference
    OutputExpression::ReadVar(Box::new_in(
        ReadVarExpr { name: conditional_factory_var, source_span: None },
        allocator,
    ))
}

/// Compiles an inherited factory using the IIFE memoization pattern.
///
/// Generates:
/// ```javascript
/// /*@__PURE__*/ (() => {
///   let ɵMyClass_BaseFactory;
///   return function MyClass_Factory(__ngFactoryType__) {
///     return (ɵMyClass_BaseFactory || (ɵMyClass_BaseFactory = ɵɵgetInheritedFactory(MyClass)))(__ngFactoryType__ || MyClass);
///   };
/// })()
/// ```
///
/// See: packages/compiler/src/render3/r3_factory.ts:160-193
fn compile_inherited_factory<'a>(
    allocator: &'a Allocator,
    base: &crate::factory::metadata::R3ConstructorFactoryMetadata<'a>,
    factory_name: &'a str,
) -> FactoryCompileResult<'a> {
    // Create base factory variable name: ɵMyClass_BaseFactory
    let base_factory_var_name =
        Ident::from_in(format!("ɵ{}_BaseFactory", base.name).as_str(), allocator);
    let factory_type_param = Ident::from("__ngFactoryType__");

    // Create ɵɵgetInheritedFactory(MyClass) call
    let get_inherited_factory_call = {
        let fn_expr = OutputExpression::ReadProp(Box::new_in(
            ReadPropExpr {
                receiver: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Ident::from("i0"), source_span: None },
                        allocator,
                    )),
                    allocator,
                ),
                name: Ident::from(Identifiers::GET_INHERITED_FACTORY),
                optional: false,
                source_span: None,
            },
            allocator,
        ));

        let mut args = Vec::new_in(allocator);
        args.push(base.type_expr.clone_in(allocator));

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
    };

    // Create assignment: ɵMyClass_BaseFactory = ɵɵgetInheritedFactory(MyClass)
    let assignment = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Assign,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: base_factory_var_name.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(get_inherited_factory_call, allocator),
            source_span: None,
        },
        allocator,
    ));

    // Create memoization pattern: baseFactoryVar || (baseFactoryVar = ɵɵgetInheritedFactory(...))
    let memoized_factory = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: base_factory_var_name.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(assignment, allocator),
            source_span: None,
        },
        allocator,
    ));

    // Create (__ngFactoryType__ || MyClass)
    let type_for_ctor = OutputExpression::BinaryOperator(Box::new_in(
        BinaryOperatorExpr {
            operator: BinaryOperator::Or,
            lhs: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: factory_type_param.clone(), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            rhs: Box::new_in(base.type_expr.clone_in(allocator), allocator),
            source_span: None,
        },
        allocator,
    ));

    // Create the factory call: (memoizedFactory)(__ngFactoryType__ || MyClass)
    let mut factory_call_args = Vec::new_in(allocator);
    factory_call_args.push(type_for_ctor);

    let factory_call = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(memoized_factory, allocator),
            args: factory_call_args,
            pure: false,
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    // Create return statement for inner function
    let mut inner_body: Vec<'a, OutputStatement<'a>> = Vec::new_in(allocator);
    inner_body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: factory_call, source_span: None },
        allocator,
    )));

    // Create inner function: function MyClass_Factory(__ngFactoryType__) { ... }
    let mut inner_params = Vec::new_in(allocator);
    inner_params.push(FnParam { name: factory_type_param });

    let inner_fn = OutputExpression::Function(Box::new_in(
        FunctionExpr {
            name: Some(Ident::from(factory_name)),
            params: inner_params,
            statements: inner_body,
            source_span: None,
        },
        allocator,
    ));

    // Create IIFE body: let ɵMyClass_BaseFactory; return function...;
    let mut iife_body: Vec<'a, OutputStatement<'a>> = Vec::new_in(allocator);

    // Declaration: let ɵMyClass_BaseFactory;
    iife_body.push(OutputStatement::DeclareVar(Box::new_in(
        DeclareVarStmt {
            name: base_factory_var_name,
            value: None,
            modifiers: StmtModifier::NONE,
            leading_comment: None,
            source_span: None,
        },
        allocator,
    )));

    // Return the inner function
    iife_body.push(OutputStatement::Return(Box::new_in(
        ReturnStatement { value: inner_fn, source_span: None },
        allocator,
    )));

    // Create arrow function IIFE: () => { let x; return function...; }
    let arrow_fn = OutputExpression::ArrowFunction(Box::new_in(
        ArrowFunctionExpr {
            params: Vec::new_in(allocator),
            body: ArrowFunctionBody::Statements(iife_body),
            source_span: None,
        },
        allocator,
    ));

    // Invoke the IIFE: (() => { ... })()
    let iife = OutputExpression::InvokeFunction(Box::new_in(
        InvokeFunctionExpr {
            fn_expr: Box::new_in(arrow_fn, allocator),
            args: Vec::new_in(allocator),
            pure: true, // Mark as @__PURE__ for tree-shaking
            optional: false,
            source_span: None,
        },
        allocator,
    ));

    FactoryCompileResult { expression: iife, statements: Vec::new_in(allocator) }
}

/// Injects dependencies by creating inject calls.
fn inject_dependencies<'a>(
    allocator: &'a Allocator,
    deps: &[R3DependencyMetadata<'a>],
    target: FactoryTarget,
) -> Vec<'a, OutputExpression<'a>> {
    let mut result = Vec::new_in(allocator);
    for (index, dep) in deps.iter().enumerate() {
        result.push(compile_inject_dependency(allocator, dep, target, index));
    }
    result
}

/// Compiles a single dependency injection call.
fn compile_inject_dependency<'a>(
    allocator: &'a Allocator,
    dep: &R3DependencyMetadata<'a>,
    target: FactoryTarget,
    index: usize,
) -> OutputExpression<'a> {
    match (&dep.token, &dep.attribute_name_type) {
        (None, _) => {
            // Invalid dependency - call invalidFactoryDep(index)
            create_invalid_factory_dep_call(allocator, index)
        }
        (Some(token), None) => {
            // Regular inject call
            let mut flags = InjectFlags::DEFAULT;
            if dep.self_ {
                flags |= InjectFlags::SELF;
            }
            if dep.skip_self {
                flags |= InjectFlags::SKIP_SELF;
            }
            if dep.host {
                flags |= InjectFlags::HOST;
            }
            if dep.optional {
                flags |= InjectFlags::OPTIONAL;
            }
            if target == FactoryTarget::Pipe {
                flags |= InjectFlags::FOR_PIPE;
            }

            let inject_fn = get_inject_fn(target);
            let mut args = Vec::new_in(allocator);
            args.push(token.clone_in(allocator));

            if flags != InjectFlags::DEFAULT || dep.optional {
                args.push(OutputExpression::Literal(Box::new_in(
                    LiteralExpr { value: LiteralValue::Number(flags as f64), source_span: None },
                    allocator,
                )));
            }

            create_import_call(allocator, inject_fn, args)
        }
        (Some(token), Some(_attr_type)) => {
            // Attribute injection
            let mut args = Vec::new_in(allocator);
            args.push(token.clone_in(allocator));
            create_import_call(allocator, Identifiers::INJECT_ATTRIBUTE, args)
        }
    }
}

/// Gets the inject function name based on target.
fn get_inject_fn(target: FactoryTarget) -> &'static str {
    match target {
        FactoryTarget::Component | FactoryTarget::Directive | FactoryTarget::Pipe => {
            Identifiers::DIRECTIVE_INJECT
        }
        FactoryTarget::NgModule | FactoryTarget::Injectable => Identifiers::INJECT,
    }
}

/// Creates an import expression call: i0.ɵɵinject(args...)
fn create_import_call<'a>(
    allocator: &'a Allocator,
    name: &'static str,
    args: Vec<'a, OutputExpression<'a>>,
) -> OutputExpression<'a> {
    let fn_expr = OutputExpression::ReadProp(Box::new_in(
        ReadPropExpr {
            receiver: Box::new_in(
                OutputExpression::ReadVar(Box::new_in(
                    ReadVarExpr { name: Ident::from("i0"), source_span: None },
                    allocator,
                )),
                allocator,
            ),
            name: Ident::from(name),
            optional: false,
            source_span: None,
        },
        allocator,
    ));

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

/// Creates i0.ɵɵinvalidFactory() call.
fn create_invalid_factory_call<'a>(allocator: &'a Allocator) -> OutputExpression<'a> {
    create_import_call(allocator, Identifiers::INVALID_FACTORY, Vec::new_in(allocator))
}

/// Creates i0.ɵɵinvalidFactoryDep(index) call.
fn create_invalid_factory_dep_call<'a>(
    allocator: &'a Allocator,
    index: usize,
) -> OutputExpression<'a> {
    let mut args = Vec::new_in(allocator);
    args.push(OutputExpression::Literal(Box::new_in(
        LiteralExpr { value: LiteralValue::Number(index as f64), source_span: None },
        allocator,
    )));
    create_import_call(allocator, Identifiers::INVALID_FACTORY_DEP, args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::factory::metadata::R3ConstructorFactoryMetadata;
    use crate::output::emitter::JsEmitter;

    #[test]
    fn test_compile_simple_factory() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("TestClass"), source_span: None },
            &allocator,
        ));

        let meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
            name: Ident::from("TestClass"),
            type_expr: type_expr.clone_in(&allocator),
            type_decl: type_expr,
            type_argument_count: 0,
            deps: R3FactoryDeps::Valid(Vec::new_in(&allocator)),
            target: FactoryTarget::Pipe,
        });

        let result = compile_factory_function(&allocator, &meta, "TestClass_Factory");
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("TestClass_Factory"));
        assert!(output.contains("__ngFactoryType__"));
    }

    #[test]
    fn test_compile_factory_with_deps() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("MyPipe"), source_span: None },
            &allocator,
        ));

        let dep_token = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("SomeService"), source_span: None },
            &allocator,
        ));

        let mut deps = Vec::new_in(&allocator);
        deps.push(R3DependencyMetadata::simple(dep_token));

        let meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
            name: Ident::from("MyPipe"),
            type_expr: type_expr.clone_in(&allocator),
            type_decl: type_expr,
            type_argument_count: 0,
            deps: R3FactoryDeps::Valid(deps),
            target: FactoryTarget::Pipe,
        });

        let result = compile_factory_function(&allocator, &meta, "MyPipe_Factory");
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("MyPipe_Factory"));
        assert!(output.contains("directiveInject")); // Uses directiveInject for pipes
    }

    #[test]
    fn test_compile_invalid_deps_factory() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("BrokenClass"), source_span: None },
            &allocator,
        ));

        let meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
            name: Ident::from("BrokenClass"),
            type_expr: type_expr.clone_in(&allocator),
            type_decl: type_expr,
            type_argument_count: 0,
            deps: R3FactoryDeps::Invalid,
            target: FactoryTarget::Injectable,
        });

        let result = compile_factory_function(&allocator, &meta, "BrokenClass_Factory");
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        assert!(output.contains("invalidFactory"));
    }

    #[test]
    fn test_compile_inherited_factory() {
        let allocator = Allocator::default();
        let type_expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Ident::from("ChildClass"), source_span: None },
            &allocator,
        ));

        // R3FactoryDeps::None indicates no constructor, use inherited factory
        let meta = R3FactoryMetadata::Constructor(R3ConstructorFactoryMetadata {
            name: Ident::from("ChildClass"),
            type_expr: type_expr.clone_in(&allocator),
            type_decl: type_expr,
            type_argument_count: 0,
            deps: R3FactoryDeps::None,
            target: FactoryTarget::Component,
        });

        let result = compile_factory_function(&allocator, &meta, "ChildClass_Factory");
        let emitter = JsEmitter::new();
        let output = emitter.emit_expression(&result.expression);

        // Should have IIFE pattern
        assert!(output.contains("()"), "Should be wrapped in IIFE");

        // Should have base factory variable declaration
        assert!(output.contains("ɵChildClass_BaseFactory"), "Should declare base factory variable");

        // Should have getInheritedFactory call
        assert!(
            output.contains("getInheritedFactory") || output.contains("ɵɵgetInheritedFactory"),
            "Should call getInheritedFactory"
        );

        // Should have the factory function name
        assert!(output.contains("ChildClass_Factory"), "Should have factory function name");
    }
}

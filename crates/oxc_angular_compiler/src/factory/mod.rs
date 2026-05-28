//! Factory function compilation module.
//!
//! This module provides the shared factory function generation used by
//! Pipe, Directive, Component, Injectable, and NgModule compilation.
//!
//! Ported from Angular's `render3/r3_factory.ts`.

mod compiler;
mod metadata;

pub use compiler::{
    FactoryCompileResult, InjectFlags, compile_factory_function, create_invalid_factory_call,
};
pub use metadata::{
    FactoryTarget, R3ConstructorFactoryMetadata, R3DelegatedFnOrClassMetadata,
    R3DependencyMetadata, R3ExpressionFactoryMetadata, R3FactoryDelegateType, R3FactoryDeps,
    R3FactoryMetadata,
};

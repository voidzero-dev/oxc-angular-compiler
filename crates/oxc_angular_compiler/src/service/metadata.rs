//! Service metadata structures.
//!
//! Ported from Angular's `service_compiler.ts:R3ServiceMetadata`.

use oxc_str::Ident;

use crate::output::ast::OutputExpression;

/// Metadata needed to compile a `@Service` decorator (Angular v22+).
///
/// Corresponds to Angular's `R3ServiceMetadata` interface in
/// `packages/compiler/src/service_compiler.ts`. Intentionally narrower than
/// `R3InjectableMetadata`: services don't support `providedIn` or the
/// `useClass`/`useFactory`/`useValue`/`useExisting` provider variants, and
/// constructor DI is resolved via `inject()` calls in the constructor body
/// rather than the ɵfac.
#[derive(Debug)]
pub struct R3ServiceMetadata<'a> {
    /// Name of the service type.
    pub name: Ident<'a>,

    /// An expression representing a reference to the service class.
    pub r#type: OutputExpression<'a>,

    /// Number of generic type parameters of the type itself.
    pub type_argument_count: u32,

    /// Whether the service is auto-provided in the root injector.
    ///
    /// `None` means the user didn't specify a value (defaults to `true` at
    /// runtime). `Some(false)` is the only value that gets emitted to the
    /// definition map — matching upstream's behavior of only emitting the
    /// field when explicitly disabled.
    pub auto_provided: Option<bool>,

    /// User-supplied factory expression from `@Service({factory: ...})`.
    ///
    /// `None` means default to `MyService.ɵfac`. `Some(expr)` produces an
    /// arrow wrapper `() => expr()` per upstream `service_compiler.ts:35-42`.
    pub factory: Option<OutputExpression<'a>>,
}

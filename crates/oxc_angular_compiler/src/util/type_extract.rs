//! Type extraction helpers shared between DI token resolution and class metadata.
//!
//! Mirrors Angular's `typeReferenceToExpression` in
//! `packages/compiler-cli/src/ngtsc/reflection/src/typescript.ts`, which filters
//! `null` literal type nodes out of a union before resolving the remaining type
//! as an injection token.

use oxc_ast::ast::TSType;

/// Resolves a TypeScript type annotation to the concrete type usable as a DI
/// injection token.
///
/// When the annotation is a union, `null` keyword variants are filtered out;
/// if exactly one non-null type remains, that type is returned. Otherwise
/// (no remaining types, or two or more remaining types) the result is `None`,
/// matching the reference compiler's "unresolvable token" behavior.
///
/// Parenthesized type nodes (`(T)`) are transparently unwrapped before union
/// handling — oxc preserves them in the AST and the reference compiler treats
/// them as the inner type for token resolution.
///
/// `undefined` is intentionally **not** stripped — this matches Angular's
/// reference compiler, where `T | undefined` remains ambiguous for token
/// resolution while `T | null` is the canonical optional-DI pattern.
///
/// Non-union, non-parenthesized types are returned as-is.
pub fn resolve_di_token_type<'a, 'b>(ts_type: &'b TSType<'a>) -> Option<&'b TSType<'a>> {
    match ts_type {
        TSType::TSParenthesizedType(paren) => resolve_di_token_type(&paren.type_annotation),
        TSType::TSUnionType(union) => {
            let mut non_null = union.types.iter().filter(|t| {
                // Unwrap parens inside the union so `MyService | (null)` and
                // `(MyService) | null` both reduce to `MyService`.
                !matches!(unwrap_parens(t), TSType::TSNullKeyword(_))
            });
            let first = non_null.next()?;
            if non_null.next().is_some() {
                // More than one non-null type — ambiguous for token resolution.
                return None;
            }
            // Recurse so nested cases like `(MyService | null) | null` collapse.
            resolve_di_token_type(first)
        }
        _ => Some(ts_type),
    }
}

/// Strip nested `TSParenthesizedType` wrappers, returning the underlying type.
fn unwrap_parens<'a, 'b>(ts_type: &'b TSType<'a>) -> &'b TSType<'a> {
    let mut current = ts_type;
    while let TSType::TSParenthesizedType(paren) = current {
        current = &paren.type_annotation;
    }
    current
}

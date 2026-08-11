//! Elide Angular metadata calls.
//!
//! This transformation removes Angular metadata calls that are only needed
//! for debugging and development. Removing them in production builds
//! reduces bundle size and enables better tree-shaking.
//!
//! ## Removed Calls
//!
//! - `ɵsetClassMetadata(Component, [...], null, null)` - Component metadata
//! - `ɵsetClassMetadataAsync(Component, () => [...], (deps) => {...})` - Async metadata
//! - `ɵsetClassDebugInfo(Component, {...})` - Debug info
//!
//! These calls are typically wrapped in IIFEs that check `ngDevMode`:
//!
//! ```javascript
//! (function () {
//!     (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(MyComponent, [...]);
//! })();
//! ```
//!
//! The optimizer replaces the metadata call with `void 0`, preserving
//! the IIFE structure and any ngDevMode/ngJitMode checks.

use oxc_ast::ast::{CallExpression, Expression, ExpressionStatement, Program, Statement};

use super::Edit;

/// Names of Angular metadata functions to elide.
const METADATA_FUNCTIONS: &[&str] =
    &["ɵsetClassMetadata", "ɵsetClassMetadataAsync", "ɵsetClassDebugInfo"];

/// Transformer that removes Angular metadata calls.
pub struct ElideMetadataTransformer;

impl ElideMetadataTransformer {
    /// Create a new metadata elision transformer.
    pub fn new() -> Self {
        Self
    }

    /// Transform the program by collecting edits to remove metadata calls.
    pub fn transform(&self, program: &Program<'_>, _source: &str) -> Vec<Edit> {
        let mut edits = Vec::new();

        for stmt in &program.body {
            self.check_statement(stmt, &mut edits);
        }

        edits
    }

    /// Check a statement for metadata calls to replace with void 0.
    fn check_statement(&self, stmt: &Statement<'_>, edits: &mut Vec<Edit>) {
        if let Statement::ExpressionStatement(expr_stmt) = stmt {
            self.check_expression_statement(expr_stmt, edits);
        }
    }

    /// Check an expression statement for metadata calls.
    fn check_expression_statement(&self, stmt: &ExpressionStatement<'_>, edits: &mut Vec<Edit>) {
        match &stmt.expression {
            // Direct call: ɵsetClassMetadata(...)
            Expression::CallExpression(call) => {
                // Check for direct metadata call
                if self.is_metadata_call(call) {
                    edits.push(Edit::replace(call.span.start, call.span.end, "void 0".to_string()));
                    return;
                }

                // Check for IIFE containing metadata
                self.check_iife_for_metadata(call, edits);
            }

            // Logical expression pattern: (typeof ngDevMode === "undefined" || ngDevMode) && ɵsetClassMetadata(...)
            Expression::LogicalExpression(logical) => {
                self.check_logical_for_metadata(logical, edits);
            }

            // Sequence expression: (0, i0.ɵsetClassMetadata)(...)
            Expression::SequenceExpression(seq) => {
                // Check if this is a sequence that leads to a metadata call
                // Pattern: (0, i0.ɵsetClassMetadata)(Component, ...)
                if let Some(last) = seq.expressions.last() {
                    if let Expression::StaticMemberExpression(member) = last {
                        if METADATA_FUNCTIONS.contains(&member.property.name.as_str()) {
                            edits.push(Edit::replace(
                                stmt.span.start,
                                stmt.span.end,
                                "void 0;".to_string(),
                            ));
                        }
                    }
                }
            }

            _ => {}
        }
    }

    /// Check a logical expression for metadata calls.
    fn check_logical_for_metadata(
        &self,
        logical: &oxc_ast::ast::LogicalExpression<'_>,
        edits: &mut Vec<Edit>,
    ) {
        match &logical.right {
            Expression::CallExpression(call) => {
                if self.is_metadata_call(call) {
                    edits.push(Edit::replace(call.span.start, call.span.end, "void 0".to_string()));
                }
            }
            Expression::LogicalExpression(inner) => {
                // Nested logical expression
                self.check_logical_for_metadata(inner, edits);
            }
            _ => {}
        }
    }

    /// Check an IIFE for metadata calls.
    fn check_iife_for_metadata(&self, call: &CallExpression<'_>, edits: &mut Vec<Edit>) {
        // Handle: (function() { ... })()
        if let Expression::ParenthesizedExpression(paren) = &call.callee {
            match &paren.expression {
                Expression::FunctionExpression(func) => {
                    if let Some(body) = &func.body {
                        self.check_function_body_for_metadata(&body.statements, edits);
                    }
                }
                Expression::ArrowFunctionExpression(arrow) => {
                    if let Some(body) = arrow.get_function_body() {
                        self.check_function_body_for_metadata(&body.statements, edits);
                    }
                    // Expression-body arrows have no statements to scan for metadata.
                }
                _ => {}
            }
        }
    }

    /// Check function body statements for metadata calls.
    fn check_function_body_for_metadata(
        &self,
        statements: &[Statement<'_>],
        edits: &mut Vec<Edit>,
    ) {
        for stmt in statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt {
                self.check_expression_for_metadata(&expr_stmt.expression, edits);
            }
        }
    }

    /// Check an expression for metadata calls and add appropriate edits.
    fn check_expression_for_metadata(&self, expr: &Expression<'_>, edits: &mut Vec<Edit>) {
        match expr {
            Expression::CallExpression(call) => {
                if self.is_metadata_call(call) {
                    edits.push(Edit::replace(call.span.start, call.span.end, "void 0".to_string()));
                }
            }
            Expression::LogicalExpression(logical) => {
                self.check_logical_for_metadata(logical, edits);
            }
            _ => {}
        }
    }

    /// Check if a call expression is a metadata function call.
    fn is_metadata_call(&self, call: &CallExpression<'_>) -> bool {
        let callee_name = self.get_callee_name(call);
        callee_name.is_some_and(|name| METADATA_FUNCTIONS.contains(&name))
    }

    /// Get the name of the callee function.
    fn get_callee_name<'a>(&self, call: &'a CallExpression<'a>) -> Option<&'a str> {
        match &call.callee {
            // Direct call: ɵsetClassMetadata(...)
            Expression::Identifier(ident) => Some(ident.name.as_str()),

            // Namespace call: i0.ɵsetClassMetadata(...)
            Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),

            // Computed member: i0["ɵsetClassMetadata"](...)
            Expression::ComputedMemberExpression(member) => {
                if let Expression::StringLiteral(lit) = &member.expression {
                    Some(lit.value.as_str())
                } else {
                    None
                }
            }

            _ => None,
        }
    }
}

impl Default for ElideMetadataTransformer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    use super::*;

    fn parse_and_transform(code: &str) -> Vec<Edit> {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let result = Parser::new(&allocator, code, source_type).parse();
        let transformer = ElideMetadataTransformer::new();
        transformer.transform(&result.program, code)
    }

    #[test]
    fn test_direct_metadata_call() {
        let code = r#"ɵsetClassMetadata(MyComponent, [], null, null);"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].start, 0);
    }

    #[test]
    fn test_namespaced_metadata_call() {
        let code = r#"i0.ɵsetClassMetadata(MyComponent, [], null, null);"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn test_iife_with_ngdevmode_check() {
        let code = r#"(function () {
            (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(MyComponent, []);
        })();"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn test_arrow_iife_with_metadata() {
        let code = r#"(() => {
            (typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(MyComponent, []);
        })();"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn test_set_class_debug_info() {
        let code = r#"i0.ɵsetClassDebugInfo(MyComponent, {});"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn test_set_class_metadata_async() {
        let code = r#"i0.ɵsetClassMetadataAsync(MyComponent, () => [], (deps) => {});"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
    }

    #[test]
    fn test_no_metadata_call() {
        let code = r#"console.log("hello");"#;
        let edits = parse_and_transform(code);
        assert!(edits.is_empty());
    }

    #[test]
    fn test_logical_and_pattern() {
        let code = r#"(typeof ngDevMode === "undefined" || ngDevMode) && i0.ɵsetClassMetadata(MyComponent, []);"#;
        let edits = parse_and_transform(code);
        assert_eq!(edits.len(), 1);
    }
}

//! Mark top-level calls as pure.
//!
//! This transformation adds `/* @__PURE__ */` annotations to top-level
//! function calls and `new` expressions. This hints to bundlers that
//! these calls have no side effects and can be removed if the result
//! is unused.
//!
//! ## What Gets Marked
//!
//! - Top-level function calls: `foo()`
//! - Top-level `new` expressions: `new Foo()`
//! - Variable initializers: `const x = foo()`
//!
//! ## What Gets Skipped
//!
//! - Calls inside functions (not top-level)
//! - tslib helper calls (`__decorate`, `__param`, etc.)
//! - Babel helper calls (`_classCallCheck`, etc.)
//! - Calls already marked with `/* @__PURE__ */`

use oxc_ast::ast::{Declaration, Expression, Program, Statement, VariableDeclaration};

use super::Edit;

/// tslib helpers that should not be marked as pure.
/// These are used by TypeScript for decorator metadata and async/await.
const TSLIB_HELPERS: &[&str] = &[
    "__decorate",
    "__param",
    "__metadata",
    "__awaiter",
    "__generator",
    "__exportStar",
    "__values",
    "__read",
    "__spread",
    "__spreadArrays",
    "__spreadArray",
    "__await",
    "__asyncGenerator",
    "__asyncDelegator",
    "__asyncValues",
    "__makeTemplateObject",
    "__importStar",
    "__importDefault",
    "__classPrivateFieldGet",
    "__classPrivateFieldSet",
    "__classPrivateFieldIn",
    "__createBinding",
    "__esDecorate",
    "__runInitializers",
];

/// Babel helpers that should not be marked as pure.
const BABEL_HELPERS: &[&str] = &[
    "_classCallCheck",
    "_createClass",
    "_defineProperty",
    "_extends",
    "_inherits",
    "_possibleConstructorReturn",
    "_getPrototypeOf",
    "_setPrototypeOf",
    "_slicedToArray",
    "_toConsumableArray",
    "_objectSpread",
    "_objectSpread2",
    "_objectWithoutProperties",
    "_objectWithoutPropertiesLoose",
    "_assertThisInitialized",
    "_typeof",
];

/// The pure annotation string.
const PURE_ANNOTATION: &str = "/* @__PURE__ */ ";

/// The pure annotation for IIFEs (uses #__PURE__ format to match Angular's output).
const PURE_IIFE_ANNOTATION: &str = "/*#__PURE__*/";

/// Transformer that marks top-level calls as pure.
pub struct MarkTopLevelPureTransformer;

impl MarkTopLevelPureTransformer {
    /// Create a new pure annotation transformer.
    pub fn new() -> Self {
        Self
    }

    /// Transform the program by collecting edits to add pure annotations.
    pub fn transform(&self, program: &Program<'_>, source: &str) -> Vec<Edit> {
        let mut edits = Vec::new();

        for stmt in &program.body {
            self.visit_top_level_statement(stmt, source, &mut edits);
        }

        edits
    }

    /// Visit a top-level statement.
    fn visit_top_level_statement(&self, stmt: &Statement<'_>, source: &str, edits: &mut Vec<Edit>) {
        match stmt {
            // Expression statement: foo();
            Statement::ExpressionStatement(expr_stmt) => {
                self.maybe_mark_expression(&expr_stmt.expression, source, edits);
            }

            // Variable declaration: const x = foo();
            Statement::VariableDeclaration(var_decl) => {
                self.visit_variable_declaration(var_decl, source, edits);
            }

            // Export default: export default foo();
            Statement::ExportDefaultDeclaration(export) => {
                if let oxc_ast::ast::ExportDefaultDeclarationKind::CallExpression(call) =
                    &export.declaration
                {
                    self.maybe_mark_call_expression(call, source, edits);
                }
            }

            // Export declaration: export const x = foo();
            Statement::ExportDeclaration(export) => {
                if let Declaration::VariableDeclaration(var_decl) = &export.declaration {
                    self.visit_variable_declaration(var_decl, source, edits);
                }
            }

            _ => {}
        }
    }

    /// Visit a variable declaration.
    fn visit_variable_declaration(
        &self,
        var_decl: &VariableDeclaration<'_>,
        source: &str,
        edits: &mut Vec<Edit>,
    ) {
        for decl in &var_decl.declarations {
            if let Some(init) = &decl.init {
                self.maybe_mark_expression(init, source, edits);
            }
        }
    }

    /// Maybe mark an expression with a pure annotation.
    fn maybe_mark_expression(&self, expr: &Expression<'_>, source: &str, edits: &mut Vec<Edit>) {
        match expr {
            Expression::CallExpression(call) => {
                self.maybe_mark_call_expression(call, source, edits);
            }
            Expression::NewExpression(new_expr) => {
                self.maybe_mark_new_expression(new_expr, source, edits);
            }
            Expression::ParenthesizedExpression(paren) => {
                // Check inner expression
                self.maybe_mark_expression(&paren.expression, source, edits);
            }
            Expression::SequenceExpression(seq) => {
                // Mark the last expression in the sequence if it's a call/new
                if let Some(last) = seq.expressions.last() {
                    self.maybe_mark_expression(last, source, edits);
                }
            }
            Expression::ConditionalExpression(cond) => {
                // Mark both branches if they're calls
                self.maybe_mark_expression(&cond.consequent, source, edits);
                self.maybe_mark_expression(&cond.alternate, source, edits);
            }
            Expression::LogicalExpression(logical) => {
                // Mark the right side if it's a call
                self.maybe_mark_expression(&logical.right, source, edits);
            }
            _ => {}
        }
    }

    /// Maybe mark a call expression with a pure annotation.
    fn maybe_mark_call_expression(
        &self,
        call: &oxc_ast::ast::CallExpression<'_>,
        source: &str,
        edits: &mut Vec<Edit>,
    ) {
        // Skip if already marked as pure
        if self.is_already_marked_pure(call.span.start, source) {
            return;
        }

        // Skip helper functions
        if let Some(name) = self.get_callee_name(call) {
            if TSLIB_HELPERS.contains(&name) || BABEL_HELPERS.contains(&name) {
                return;
            }
        }

        // Check if this is an IIFE
        if self.is_iife(call) {
            // Only mark IIFEs with no arguments as pure
            // IIFEs with arguments might have side effects via those arguments
            if call.arguments.is_empty() {
                edits.push(Edit::insert(call.span.start, PURE_IIFE_ANNOTATION.to_string()));
            }
            return;
        }

        // Add pure annotation
        edits.push(Edit::insert(call.span.start, PURE_ANNOTATION.to_string()));
    }

    /// Maybe mark a new expression with a pure annotation.
    fn maybe_mark_new_expression(
        &self,
        new_expr: &oxc_ast::ast::NewExpression<'_>,
        source: &str,
        edits: &mut Vec<Edit>,
    ) {
        // Skip if already marked as pure
        if self.is_already_marked_pure(new_expr.span.start, source) {
            return;
        }

        // Add pure annotation
        edits.push(Edit::insert(new_expr.span.start, PURE_ANNOTATION.to_string()));
    }

    /// Check if a position is already marked with /* @__PURE__ */.
    fn is_already_marked_pure(&self, position: u32, source: &str) -> bool {
        let pos = position as usize;
        if pos < 15 {
            return false;
        }

        // Look backwards for the annotation
        let before = &source[..pos];
        let trimmed = before.trim_end();
        trimmed.ends_with("/* @__PURE__ */") || trimmed.ends_with("/*@__PURE__*/")
    }

    /// Get the callee function name.
    fn get_callee_name<'a>(&self, call: &'a oxc_ast::ast::CallExpression<'a>) -> Option<&'a str> {
        match &call.callee {
            Expression::Identifier(ident) => Some(ident.name.as_str()),
            Expression::StaticMemberExpression(member) => Some(member.property.name.as_str()),
            _ => None,
        }
    }

    /// Check if a call is an IIFE.
    fn is_iife(&self, call: &oxc_ast::ast::CallExpression<'_>) -> bool {
        matches!(
            &call.callee,
            Expression::FunctionExpression(_)
                | Expression::ArrowFunctionExpression(_)
                | Expression::ParenthesizedExpression(_)
        )
    }
}

impl Default for MarkTopLevelPureTransformer {
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
    use crate::optimizer::apply_edits;

    fn transform(code: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::mjs();
        let result = Parser::new(&allocator, code, source_type).parse();
        let transformer = MarkTopLevelPureTransformer::new();
        let edits = transformer.transform(&result.program, code);
        apply_edits(code, edits)
    }

    #[test]
    fn test_mark_call() {
        let code = "const x = foo();";
        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ foo()"));
    }

    #[test]
    fn test_mark_new() {
        let code = "const x = new Foo();";
        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ new Foo()"));
    }

    #[test]
    fn test_skip_tslib_helper() {
        let code = "const x = __decorate([Injectable()], MyClass);";
        let result = transform(code);
        // __decorate should not be marked, but Injectable() should
        assert!(!result.contains("/* @__PURE__ */ __decorate"));
    }

    #[test]
    fn test_skip_already_marked() {
        let code = "const x = /* @__PURE__ */ foo();";
        let result = transform(code);
        // Should not add a second annotation
        assert_eq!(result.matches("@__PURE__").count(), 1);
    }

    #[test]
    fn test_expression_statement() {
        let code = "foo();";
        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ foo()"));
    }

    #[test]
    fn test_member_call() {
        let code = "const x = i0.ɵɵdefineComponent({});";
        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */"));
    }
}

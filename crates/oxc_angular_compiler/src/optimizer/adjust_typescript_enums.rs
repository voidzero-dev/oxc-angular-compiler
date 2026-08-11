//! Adjust TypeScript enum patterns for tree-shaking.
//!
//! This transformation optimizes TypeScript enum patterns to enable
//! tree-shaking. TypeScript compiles enums to IIFEs that are not
//! marked as pure, preventing bundlers from removing unused enums.
//!
//! ## Transformation
//!
//! Before (TypeScript enum output):
//! ```javascript
//! var MyEnum;
//! (function(MyEnum) {
//!     MyEnum[MyEnum["Foo"] = 0] = "Foo";
//!     MyEnum[MyEnum["Bar"] = 1] = "Bar";
//! })(MyEnum || (MyEnum = {}));
//! ```
//!
//! After:
//! ```javascript
//! var MyEnum = /* @__PURE__ */ ((MyEnum) => {
//!     MyEnum[MyEnum["Foo"] = 0] = "Foo";
//!     MyEnum[MyEnum["Bar"] = 1] = "Bar";
//!     return MyEnum;
//! })(MyEnum || {});
//! ```
//!
//! The transformation:
//! 1. Converts the function expression to an arrow function
//! 2. Adds a return statement
//! 3. Combines the variable declaration and IIFE call
//! 4. Adds `/* @__PURE__ */` annotation
//! 5. Changes `(MyEnum = {})` to just `{}` since the assignment is redundant
//!
//! Also handles:
//! - Exported enums: `export var MyEnum;`
//! - Renamed variables: `var MyEnum$1;` with `(function(MyEnum) {...})`
//! - CommonJS exports: `(X || (exports.X = X = {}))`

use oxc_ast::ast::{AssignmentTarget, BindingPattern, Expression, Program, Statement};
use oxc_span::GetSpan;

use super::Edit;

/// Transformer that optimizes TypeScript enum patterns.
pub struct AdjustTypeScriptEnumsTransformer;

impl AdjustTypeScriptEnumsTransformer {
    /// Create a new enum adjustment transformer.
    pub fn new() -> Self {
        Self
    }

    /// Transform the program by collecting edits to optimize enum patterns.
    pub fn transform(&self, program: &Program<'_>, source: &str) -> Vec<Edit> {
        let mut edits = Vec::new();
        let statements = &program.body;

        // Look for the pattern: var X; (function(X) { ... })(X || (X = {}));
        let mut i = 0;
        while i < statements.len() {
            if let Some((enum_name, var_start, _var_end, is_export)) =
                self.is_enum_var_declaration(&statements[i])
            {
                // Check if the next statement is the enum IIFE
                if i + 1 < statements.len() {
                    if let Some(iife_info) =
                        self.is_enum_iife(&statements[i + 1], enum_name, source)
                    {
                        // Found the pattern - create transformation
                        if let Some(edit) = self.transform_enum_pattern(
                            enum_name, var_start, &iife_info, source, is_export,
                        ) {
                            edits.push(edit);
                            i += 2; // Skip both statements
                            continue;
                        }
                    }
                }
            }
            i += 1;
        }

        edits
    }

    /// Check if a statement is a TypeScript enum variable declaration (var X; or export var X;).
    /// Returns (enum_name, start, end, is_export).
    fn is_enum_var_declaration<'a>(
        &self,
        stmt: &'a Statement<'a>,
    ) -> Option<(&'a str, u32, u32, bool)> {
        // Check for exported var declaration (`export var X;`)
        if let Statement::ExportDeclaration(export) = stmt {
            if let oxc_ast::ast::Declaration::VariableDeclaration(var_decl) = &export.declaration {
                // Must be `var` (TypeScript uses var for enums)
                if !matches!(var_decl.kind, oxc_ast::ast::VariableDeclarationKind::Var) {
                    return None;
                }

                // Must have exactly one declaration with no initializer
                if var_decl.declarations.len() != 1 {
                    return None;
                }

                let decl = &var_decl.declarations[0];
                if decl.init.is_some() {
                    return None;
                }

                // Get the variable name
                if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                    return Some((ident.name.as_str(), export.span.start, export.span.end, true));
                }
            }
            return None;
        }

        // Check for regular var declaration
        if let Statement::VariableDeclaration(var_decl) = stmt {
            // Must be `var` (TypeScript uses var for enums)
            if !matches!(var_decl.kind, oxc_ast::ast::VariableDeclarationKind::Var) {
                return None;
            }

            // Must have exactly one declaration with no initializer
            if var_decl.declarations.len() != 1 {
                return None;
            }

            let decl = &var_decl.declarations[0];
            if decl.init.is_some() {
                return None;
            }

            // Get the variable name
            if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                return Some((ident.name.as_str(), var_decl.span.start, var_decl.span.end, false));
            }
        }

        None
    }

    /// Check if a statement is an enum IIFE pattern.
    /// Returns information about the IIFE if it matches.
    /// The enum_name can differ from the IIFE parameter name due to scope hoisting
    /// (e.g., NotificationKind$1 vs NotificationKind).
    fn is_enum_iife(
        &self,
        stmt: &Statement<'_>,
        var_name: &str,
        source: &str,
    ) -> Option<EnumIifeInfo> {
        let expr_stmt = match stmt {
            Statement::ExpressionStatement(es) => es,
            _ => return None,
        };

        let call = match &expr_stmt.expression {
            Expression::CallExpression(c) => c,
            _ => return None,
        };

        // Check if callee is a parenthesized function expression
        let func = match &call.callee {
            Expression::ParenthesizedExpression(paren) => match &paren.expression {
                Expression::FunctionExpression(f) => f,
                _ => return None,
            },
            _ => return None,
        };

        // Check that the function has exactly one parameter
        if func.params.items.len() != 1 {
            return None;
        }

        let param = &func.params.items[0];
        let param_name = match &param.pattern {
            BindingPattern::BindingIdentifier(ident) => ident.name.as_str(),
            _ => return None,
        };

        // The var name should either match the param name or be a renamed version
        // (e.g., MyEnum$1 should match MyEnum parameter)
        if !self.names_match(var_name, param_name) {
            return None;
        }

        // Check that the call has exactly one argument
        if call.arguments.len() != 1 {
            return None;
        }

        let arg = &call.arguments[0];
        let arg_info = self.check_enum_default_arg(arg, var_name)?;

        // Skip unsupported patterns (old CommonJS with exports assignment on left)
        if arg_info.unsupported {
            return None;
        }

        // Get the function body
        let body = func.body.as_ref()?;

        // Check that the body only contains safe enum assignments (no side effects)
        if !self.is_safe_enum_body(body) {
            return None;
        }

        Some(EnumIifeInfo {
            param_name: param_name.to_string(),
            func_body_start: body.span.start,
            func_body_end: body.span.end,
            iife_end: expr_stmt.span.end,
            arg_span_start: arg.span().start,
            arg_span_end: arg.span().end,
            has_exports_assignment: arg_info.has_exports,
            original_arg_text: source[arg.span().start as usize..arg.span().end as usize]
                .to_string(),
        })
    }

    /// Check if two names match, accounting for renamed variables (e.g., MyEnum$1 matches MyEnum).
    fn names_match(&self, var_name: &str, param_name: &str) -> bool {
        if var_name == param_name {
            return true;
        }
        // Check if var_name is a renamed version (param_name$N pattern)
        if let Some(stripped) = var_name.strip_suffix(|c: char| c.is_ascii_digit() || c == '$') {
            // Handle NotificationKind$1 -> NotificationKind$ -> NotificationKind
            let base =
                stripped.trim_end_matches('$').trim_end_matches(|c: char| c.is_ascii_digit());
            let final_base = base.trim_end_matches('$');
            return final_base == param_name || stripped.trim_end_matches('$') == param_name;
        }
        false
    }

    /// Check if an argument is the enum default pattern and return info about it.
    fn check_enum_default_arg(
        &self,
        arg: &oxc_ast::ast::Argument<'_>,
        var_name: &str,
    ) -> Option<ArgInfo> {
        let expr = match arg {
            oxc_ast::ast::Argument::SpreadElement(_) => return None,
            _ => arg.to_expression(),
        };

        // Check for: VarName || (VarName = {}) or VarName || (exports.VarName = VarName = {})
        if let Expression::LogicalExpression(logical) = expr {
            if !matches!(logical.operator, oxc_ast::ast::LogicalOperator::Or) {
                return None;
            }

            // Left should be identifier: VarName
            if let Expression::Identifier(ident) = &logical.left {
                if ident.name.as_str() != var_name {
                    return None;
                }
            } else {
                return None;
            }

            // Right should be an assignment pattern
            let right = match &logical.right {
                Expression::ParenthesizedExpression(paren) => &paren.expression,
                other => other,
            };

            if let Expression::AssignmentExpression(assign) = right {
                return self.check_assignment_pattern(assign, var_name);
            }
        }

        None
    }

    /// Check if the enum body contains only safe assignments (no side effects).
    fn is_safe_enum_body(&self, body: &oxc_ast::ast::FunctionBody<'_>) -> bool {
        for stmt in &body.statements {
            if let Statement::ExpressionStatement(expr_stmt) = stmt {
                if !self.is_safe_enum_assignment(&expr_stmt.expression) {
                    return false;
                }
            }
            // Other statement types (return, etc.) are allowed
        }
        true
    }

    /// Check if an expression is a safe enum assignment.
    /// Safe assignments have the form: Enum[key] = value where value is safe.
    fn is_safe_enum_assignment(&self, expr: &Expression<'_>) -> bool {
        if let Expression::AssignmentExpression(assign) = expr {
            // Check the right-hand side (the assigned value)
            return self.is_safe_value(&assign.right);
        }
        // Not an assignment - might be okay if it's something benign
        self.is_safe_value(expr)
    }

    /// Check if a value is safe (no side effects).
    fn is_safe_value(&self, expr: &Expression<'_>) -> bool {
        match expr {
            // Literals are safe
            Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::NullLiteral(_) => true,

            // Identifiers are safe (they just reference values)
            Expression::Identifier(_) => true,

            // Template literals without expressions are safe
            Expression::TemplateLiteral(t) => t.expressions.is_empty(),

            // Assignments to computed properties need to check the value
            // E.g., Enum[Enum["X"] = 0] = "X" - need to check both values
            Expression::AssignmentExpression(assign) => self.is_safe_value(&assign.right),

            // Member expressions (Enum["X"], Enum.X) are safe reads
            Expression::StaticMemberExpression(_) | Expression::ComputedMemberExpression(_) => true,

            // Parenthesized expressions - check inner
            Expression::ParenthesizedExpression(p) => self.is_safe_value(&p.expression),

            // Unary expressions on safe values (like -1, +0, !x) are safe
            Expression::UnaryExpression(u) => self.is_safe_value(&u.argument),

            // Binary expressions on safe values are safe
            Expression::BinaryExpression(b) => {
                self.is_safe_value(&b.left) && self.is_safe_value(&b.right)
            }

            // Everything else (calls, new, etc.) might have side effects
            _ => false,
        }
    }

    /// Check assignment pattern in the IIFE argument.
    fn check_assignment_pattern(
        &self,
        assign: &oxc_ast::ast::AssignmentExpression<'_>,
        var_name: &str,
    ) -> Option<ArgInfo> {
        // Pattern 1: VarName = {} (simple)
        // Pattern 2: exports.VarName = VarName = {} (CommonJS 5.1+)
        // Pattern 3: VarName = exports.VarName || (exports.VarName = {}) (old CommonJS, unsupported)

        match &assign.left {
            AssignmentTarget::AssignmentTargetIdentifier(target) => {
                // Simple pattern: VarName = {}
                if target.name.as_str() != var_name {
                    return None;
                }
                if let Expression::ObjectExpression(obj) = &assign.right {
                    if obj.properties.is_empty() {
                        return Some(ArgInfo { has_exports: false, unsupported: false });
                    }
                }
                None
            }
            AssignmentTarget::StaticMemberExpression(member) => {
                // Old CommonJS pattern: exports.VarName = ... (unsupported if the value is complex)
                if let Expression::Identifier(obj) = &member.object {
                    if obj.name.as_str() == "exports" {
                        // Check the right side - could be (VarName = {}) for the newer pattern
                        if let Expression::AssignmentExpression(inner) = &assign.right {
                            if let AssignmentTarget::AssignmentTargetIdentifier(inner_target) =
                                &inner.left
                            {
                                if inner_target.name.as_str() == var_name {
                                    if let Expression::ObjectExpression(obj) = &inner.right {
                                        if obj.properties.is_empty() {
                                            // CommonJS 5.1+ pattern: exports.VarName = VarName = {}
                                            return Some(ArgInfo {
                                                has_exports: true,
                                                unsupported: false,
                                            });
                                        }
                                    }
                                }
                            }
                        }
                        // Old pattern - unsupported
                        return Some(ArgInfo { has_exports: true, unsupported: true });
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Transform an enum pattern to the optimized form.
    fn transform_enum_pattern(
        &self,
        var_name: &str,
        var_start: u32,
        iife_info: &EnumIifeInfo,
        source: &str,
        is_export: bool,
    ) -> Option<Edit> {
        let iife_end = iife_info.iife_end;

        // Get the function body content (without braces)
        let body_start = (iife_info.func_body_start + 1) as usize;
        let body_end = (iife_info.func_body_end - 1) as usize;
        let body_content = &source[body_start..body_end];

        // Use the IIFE parameter name for the return statement
        let param_name = &iife_info.param_name;

        // Build the optimized code
        let mut optimized = String::new();

        if is_export {
            optimized.push_str("export ");
        }
        optimized.push_str("var ");
        optimized.push_str(var_name);
        optimized.push_str(" = /* @__PURE__ */ ((");
        optimized.push_str(param_name);
        optimized.push_str(") => {");
        optimized.push_str(body_content);
        optimized.push_str("return ");
        optimized.push_str(param_name);
        optimized.push_str(";\n})(");

        // Preserve the original argument for CommonJS patterns
        if iife_info.has_exports_assignment {
            optimized.push_str(&iife_info.original_arg_text);
        } else {
            optimized.push_str(var_name);
            optimized.push_str(" || {}");
        }
        optimized.push(')');

        // Preserve trailing semicolon if present
        let source_at_end = source.get((iife_end - 1) as usize..iife_end as usize);
        if source_at_end == Some(";") {
            optimized.push(';');
        }

        Some(Edit::replace(var_start, iife_end, optimized))
    }
}

impl Default for AdjustTypeScriptEnumsTransformer {
    fn default() -> Self {
        Self::new()
    }
}

/// Information about an enum IIFE pattern.
struct EnumIifeInfo {
    /// The IIFE parameter name (may differ from variable name due to scope hoisting).
    param_name: String,
    /// Start of the function body (including `{`).
    func_body_start: u32,
    /// End of the function body (including `}`).
    func_body_end: u32,
    /// End of the IIFE expression statement.
    iife_end: u32,
    /// Start of the argument span.
    #[allow(dead_code)]
    arg_span_start: u32,
    /// End of the argument span.
    #[allow(dead_code)]
    arg_span_end: u32,
    /// Whether the argument has exports assignment.
    has_exports_assignment: bool,
    /// Original argument text to preserve for CommonJS patterns.
    original_arg_text: String,
}

/// Information about an enum IIFE argument.
struct ArgInfo {
    /// Whether this pattern has exports assignment.
    has_exports: bool,
    /// Whether this pattern is unsupported (old CommonJS).
    unsupported: bool,
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
        let transformer = AdjustTypeScriptEnumsTransformer::new();
        let edits = transformer.transform(&result.program, code);
        apply_edits(code, edits)
    }

    #[test]
    fn test_transform_basic_enum() {
        let code = r#"var MyEnum;
(function(MyEnum) {
    MyEnum[MyEnum["Foo"] = 0] = "Foo";
    MyEnum[MyEnum["Bar"] = 1] = "Bar";
})(MyEnum || (MyEnum = {}));"#;

        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */"));
        assert!(result.contains("var MyEnum = "));
        assert!(result.contains("return MyEnum;"));
        assert!(result.contains(")(MyEnum || {})"));
    }

    #[test]
    fn test_no_transform_non_enum() {
        let code = r#"var x = 5;
console.log(x);"#;

        let result = transform(code);
        assert!(!result.contains("/* @__PURE__ */"));
    }

    #[test]
    fn test_transform_enum_with_strings() {
        let code = r#"var Status;
(function(Status) {
    Status["Active"] = "ACTIVE";
    Status["Inactive"] = "INACTIVE";
})(Status || (Status = {}));"#;

        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */"));
    }
}

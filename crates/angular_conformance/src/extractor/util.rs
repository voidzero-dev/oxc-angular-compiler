//! Utility functions for spec extraction.
//!
//! Contains helper functions for string resolution, JSON value extraction,
//! and options parsing from AST expressions.

use oxc_ast::ast::{Argument, CallExpression, Expression};

use crate::test_case::{HtmlLexerOptions, HtmlParserOptions, LexerRange, ParseOptions};

use super::SpecExtractor;

impl SpecExtractor {
    /// Extract a string from a string literal expression
    pub(super) fn extract_string(&self, expr: &Expression<'_>) -> Option<String> {
        match expr {
            Expression::StringLiteral(lit) => Some(lit.value.to_string()),
            Expression::TemplateLiteral(lit) if lit.expressions.is_empty() => {
                // Template literal with no expressions - use cooked value (with escapes processed)
                if lit.quasis.len() == 1 {
                    lit.quasis[0].value.cooked.as_ref().map(std::string::ToString::to_string)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    /// Extract a string from an argument
    pub(super) fn extract_arg_string(&self, arg: &Argument<'_>) -> Option<String> {
        arg.as_expression().and_then(|e| self.extract_string(e))
    }

    /// Resolve a string value from an expression, including variable references
    /// This handles:
    /// - String literals: 'text'
    /// - Template literals without expressions: `text`
    /// - Variable references: identifier -> look up in pending_string_assignments
    pub(super) fn resolve_string_value(&self, arg: Option<&Argument<'_>>) -> Option<String> {
        let expr = arg?.as_expression()?;
        self.resolve_string_from_expression(expr)
    }

    /// Resolve a string value directly from an expression
    pub(super) fn resolve_string_from_expression(&self, expr: &Expression<'_>) -> Option<String> {
        match expr {
            Expression::StringLiteral(lit) => Some(lit.value.to_string()),
            Expression::TemplateLiteral(lit) if lit.expressions.is_empty() => {
                // Template literal with no expressions - use cooked value
                if lit.quasis.len() == 1 {
                    lit.quasis[0].value.cooked.as_ref().map(std::string::ToString::to_string)
                } else {
                    // Multiple quasis but no expressions means we can concatenate them
                    let parts: Vec<&str> = lit
                        .quasis
                        .iter()
                        .filter_map(|q| q.value.cooked.as_ref().map(oxc_str::Str::as_str))
                        .collect();
                    Some(parts.join(""))
                }
            }
            Expression::Identifier(id) => {
                // Look up in pending_string_assignments
                self.pending_string_assignments.get(id.name.as_str()).cloned()
            }
            Expression::BinaryExpression(bin)
                if bin.operator == oxc_ast::ast::BinaryOperator::Addition =>
            {
                // Handle string concatenation: 'a' + 'b' + 'c'
                let left = self.resolve_string_from_expression(&bin.left)?;
                let right = self.resolve_string_from_expression(&bin.right)?;
                Some(format!("{left}{right}"))
            }
            Expression::ParenthesizedExpression(paren) => {
                self.resolve_string_from_expression(&paren.expression)
            }
            _ => None,
        }
    }

    /// Get the current path for test filtering
    pub(super) fn current_path(&self) -> String {
        self.describe_stack.join("/")
    }

    /// Get function name from call expression
    pub(super) fn get_callee_name(&self, expr: &CallExpression<'_>) -> Option<String> {
        match &expr.callee {
            Expression::Identifier(id) => Some(id.name.to_string()),
            Expression::StaticMemberExpression(member) => Some(member.property.name.to_string()),
            _ => None,
        }
    }

    /// Extract an array literal expression into JSON values
    /// Also handles jasmine.arrayContaining([...]) by extracting the inner array
    pub(super) fn extract_array_literal(
        &self,
        expr: &Expression<'_>,
    ) -> Option<Vec<serde_json::Value>> {
        match expr {
            Expression::ArrayExpression(arr) => {
                let mut result = Vec::new();
                for element in &arr.elements {
                    if let Some(elem_expr) = element.as_expression()
                        && let Some(value) = self.extract_json_value(elem_expr)
                    {
                        result.push(value);
                    }
                }
                Some(result)
            }
            Expression::CallExpression(call) => {
                // Handle jasmine.arrayContaining([...])
                if let Expression::StaticMemberExpression(member) = &call.callee
                    && member.property.name == "arrayContaining"
                    && let Expression::Identifier(id) = &member.object
                    && id.name == "jasmine"
                {
                    // Get the array argument
                    if let Some(arg) = call.arguments.first()
                        && let Some(inner_expr) = arg.as_expression()
                    {
                        return self.extract_array_literal(inner_expr);
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// Extract a JSON-compatible value from an expression
    #[expect(clippy::self_only_used_in_recursion)] // Method for consistency with other extraction methods
    pub(super) fn extract_json_value(&self, expr: &Expression<'_>) -> Option<serde_json::Value> {
        match expr {
            Expression::StringLiteral(lit) => {
                Some(serde_json::Value::String(lit.value.to_string()))
            }
            Expression::NumericLiteral(lit) => {
                serde_json::Number::from_f64(lit.value.into()).map(serde_json::Value::Number)
            }
            Expression::NullLiteral(_) => Some(serde_json::Value::Null),
            Expression::BooleanLiteral(lit) => Some(serde_json::Value::Bool(lit.value.into())),
            Expression::ArrayExpression(arr) => {
                let mut values = Vec::new();
                for element in &arr.elements {
                    if let Some(elem_expr) = element.as_expression()
                        && let Some(value) = self.extract_json_value(elem_expr)
                    {
                        values.push(value);
                    }
                }
                Some(serde_json::Value::Array(values))
            }
            Expression::UnaryExpression(unary) => {
                // Handle negative numbers like -1
                if unary.operator == oxc_ast::ast::UnaryOperator::UnaryNegation
                    && let Expression::NumericLiteral(lit) = &unary.argument
                {
                    return serde_json::Number::from_f64(-lit.value).map(serde_json::Value::Number);
                }
                None
            }
            Expression::BinaryExpression(bin) => {
                // Handle string concatenation like 'a' + 'b' + 'c'
                if bin.operator == oxc_ast::ast::BinaryOperator::Addition {
                    let left = self.extract_json_value(&bin.left);
                    let right = self.extract_json_value(&bin.right);
                    if let (
                        Some(serde_json::Value::String(l)),
                        Some(serde_json::Value::String(r)),
                    ) = (left, right)
                    {
                        return Some(serde_json::Value::String(format!("{l}{r}")));
                    }
                }
                None
            }
            Expression::TemplateLiteral(lit) if lit.expressions.is_empty() => {
                // Simple template literal with no expressions - use cooked value
                if lit.quasis.len() == 1 {
                    lit.quasis[0]
                        .value
                        .cooked
                        .as_ref()
                        .map(|s| serde_json::Value::String(s.to_string()))
                } else {
                    None
                }
            }
            Expression::Identifier(id) => {
                // Handle identifiers like 'null' as a string or specific enums
                Some(serde_json::Value::String(id.name.to_string()))
            }
            Expression::StaticMemberExpression(member) => {
                // Handle member expressions like BindingType.Property, ParsedEventType.Regular
                // Convert to the numeric value used in Angular
                let object_name = match &member.object {
                    Expression::Identifier(id) => Some(id.name.as_str()),
                    _ => None,
                };
                let property_name = member.property.name.as_str();

                #[expect(clippy::match_same_arms)] // Keep separate for clarity: different enums
                match (object_name, property_name) {
                    // BindingType enum values (Angular enum order: Property=0, Attribute=1, Class=2, Style=3, LegacyAnimation=4, TwoWay=5, Animation=6)
                    (Some("BindingType"), "Property") => Some(serde_json::json!(0)),
                    (Some("BindingType"), "Attribute") => Some(serde_json::json!(1)),
                    (Some("BindingType"), "Class") => Some(serde_json::json!(2)),
                    (Some("BindingType"), "Style") => Some(serde_json::json!(3)),
                    (Some("BindingType"), "LegacyAnimation") => Some(serde_json::json!(4)),
                    (Some("BindingType"), "TwoWay") => Some(serde_json::json!(5)),
                    (Some("BindingType"), "Animation") => Some(serde_json::json!(6)),
                    // ParsedEventType enum values (Angular enum order: Regular=0, LegacyAnimation=1, TwoWay=2, Animation=3)
                    (Some("ParsedEventType"), "Regular") => Some(serde_json::json!(0)),
                    (Some("ParsedEventType"), "LegacyAnimation") => Some(serde_json::json!(1)),
                    (Some("ParsedEventType"), "TwoWay") => Some(serde_json::json!(2)),
                    (Some("ParsedEventType"), "Animation") => Some(serde_json::json!(3)),
                    // Fall back to string representation
                    _ => Some(serde_json::Value::String(format!(
                        "{}.{}",
                        object_name.unwrap_or("?"),
                        property_name
                    ))),
                }
            }
            _ => None,
        }
    }

    /// Extract HTML lexer options from an object expression
    pub(super) fn extract_html_lexer_options(
        &self,
        expr: &Expression<'_>,
    ) -> Option<HtmlLexerOptions> {
        if let Expression::ObjectExpression(obj) = expr {
            let mut options = HtmlLexerOptions {
                tokenize_expansion_forms: false,
                interpolation_config: None,
                escaped_string: false,
                tokenize_blocks: None, // None means use default (true)
                leading_trivia_chars: None,
                range: None,
            };

            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    let key = match &p.key {
                        oxc_ast::ast::PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                        _ => None,
                    };

                    match key {
                        Some("tokenizeExpansionForms") => {
                            if let Expression::BooleanLiteral(b) = &p.value {
                                options.tokenize_expansion_forms = b.value;
                            }
                        }
                        Some("escapedString") => {
                            if let Expression::BooleanLiteral(b) = &p.value {
                                options.escaped_string = b.value;
                            }
                        }
                        Some("tokenizeBlocks") => {
                            if let Expression::BooleanLiteral(b) = &p.value {
                                options.tokenize_blocks = Some(b.value);
                            }
                        }
                        // Note: interpolationConfig would need to extract [start, end] tuple - skipped for now
                        Some("leadingTriviaChars") => {
                            // Extract array of single-character strings
                            if let Expression::ArrayExpression(arr) = &p.value {
                                let chars: Vec<String> = arr
                                    .elements
                                    .iter()
                                    .filter_map(|el| el.as_expression())
                                    .filter_map(|e| self.extract_string(e))
                                    .collect();
                                if !chars.is_empty() {
                                    options.leading_trivia_chars = Some(chars);
                                }
                            }
                        }
                        Some("range") => {
                            // Extract range object: {startPos, startLine, startCol, endPos}
                            if let Expression::ObjectExpression(range_obj) = &p.value {
                                let mut start_pos = None;
                                let mut start_line = None;
                                let mut start_col = None;
                                let mut end_pos = None;

                                for range_prop in &range_obj.properties {
                                    if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(rp) =
                                        range_prop
                                    {
                                        let rkey = match &rp.key {
                                            oxc_ast::ast::PropertyKey::StaticIdentifier(id) => {
                                                Some(id.name.as_str())
                                            }
                                            _ => None,
                                        };
                                        if let Expression::NumericLiteral(n) = &rp.value {
                                            match rkey {
                                                Some("startPos") => {
                                                    start_pos = Some(n.value as u32);
                                                }
                                                Some("startLine") => {
                                                    start_line = Some(n.value as u32);
                                                }
                                                Some("startCol") => {
                                                    start_col = Some(n.value as u32);
                                                }
                                                Some("endPos") => end_pos = Some(n.value as u32),
                                                _ => {}
                                            }
                                        }
                                    }
                                }

                                if let (
                                    Some(start_pos),
                                    Some(start_line),
                                    Some(start_col),
                                    Some(end_pos),
                                ) = (start_pos, start_line, start_col, end_pos)
                                {
                                    options.range = Some(LexerRange {
                                        start_pos,
                                        start_line,
                                        start_col,
                                        end_pos,
                                    });
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }

            Some(options)
        } else {
            None
        }
    }

    /// Extract HTML parser options from a parse call
    pub(super) fn extract_html_parser_options(
        &self,
        parse_call: &CallExpression<'_>,
    ) -> Option<HtmlParserOptions> {
        // Options are typically the 3rd argument: parser.parse(input, 'TestComp', { leadingTriviaChars: [...] })
        let options_arg = parse_call.arguments.get(2)?.as_expression()?;
        if let Expression::ObjectExpression(obj) = options_arg {
            let mut options = HtmlParserOptions::default();
            let mut has_options = false;

            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop
                    && let oxc_ast::ast::PropertyKey::StaticIdentifier(id) = &p.key
                    && id.name == "leadingTriviaChars"
                    && let Expression::ArrayExpression(arr) = &p.value
                {
                    let chars: Vec<String> = arr
                        .elements
                        .iter()
                        .filter_map(|e| e.as_expression())
                        .filter_map(|e| self.extract_string(e))
                        .collect();
                    if !chars.is_empty() {
                        options.leading_trivia_chars = Some(chars);
                        has_options = true;
                    }
                }
            }

            if has_options { Some(options) } else { None }
        } else {
            None
        }
    }

    /// Extract ParseOptions from an object expression
    pub(super) fn extract_parse_options(
        &self,
        expr: Option<&Expression<'_>>,
    ) -> Option<ParseOptions> {
        let expr = expr?;
        if let Expression::ObjectExpression(obj) = expr {
            let mut options = ParseOptions::default();
            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    let key = match &p.key {
                        oxc_ast::ast::PropertyKey::StaticIdentifier(id) => Some(id.name.as_str()),
                        _ => None,
                    };
                    if key == Some("preserveWhitespaces")
                        && let Expression::BooleanLiteral(b) = &p.value
                    {
                        options.preserve_whitespaces = b.value;
                    }
                }
            }
            Some(options)
        } else {
            None
        }
    }

    /// Generic helper to get a chained function call pattern: expect(funcName(...)).matcherName(...)
    pub(super) fn get_chained_function_call<'b>(
        &self,
        matcher_expr: &'b CallExpression<'b>,
        matcher_name: &str,
        func_name: &str,
    ) -> Option<&'b CallExpression<'b>> {
        if let Expression::StaticMemberExpression(member) = &matcher_expr.callee
            && member.property.name == matcher_name
            && let Expression::CallExpression(expect_call) = &member.object
            && let Some(name) = self.get_callee_name(expect_call)
            && name == "expect"
            && let Some(arg) = expect_call.arguments.first()
            && let Some(Expression::CallExpression(func_call)) = arg.as_expression()
            && let Some(fn_name) = self.get_callee_name(func_call)
            && fn_name == func_name
        {
            return Some(func_call);
        }
        None
    }
}

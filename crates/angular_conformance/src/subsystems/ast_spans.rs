use oxc_allocator::Allocator;
use oxc_angular_compiler::ast::expression::{AngularExpression, BinaryOperator};
use oxc_angular_compiler::ast::r3::{
    R3BoundText, R3BoundaryErrorBlock, R3DeferredBlock, R3Element, R3ForLoopBlock, R3IfBlock,
    R3SwitchBlock, R3Template, R3TemplateAttr, R3Visitor, visit_all,
};
use oxc_angular_compiler::parser::html::HtmlParser;
use oxc_angular_compiler::transform::html_to_r3::{HtmlToR3Transform, TransformOptions};

use super::SubsystemRunner;
use super::unparser::unparse_expression;
use crate::test_case::{ParseOptions, TestAssertion, TestResult};

/// Runner for Angular AST span conformance tests
/// Tests humanizeExpressionSource() and related span extraction
pub struct AstSpansRunner;

impl AstSpansRunner {
    pub fn new() -> Self {
        Self
    }

    /// Parse template and extract expression source spans
    fn extract_expression_sources(
        &self,
        input: &str,
        _options: Option<&ParseOptions>,
        extract_sub_expressions: bool,
        find_implicit_receiver: bool,
        template_expressions_only: bool,
        use_unparser: bool,
        unparse_full_interpolation: bool,
    ) -> Vec<Vec<String>> {
        let allocator = Allocator::default();
        let parser = HtmlParser::new(&allocator, input, "test.html");
        let html_result = parser.parse();

        // Don't bail out on parse errors - Angular's parser has error recovery
        // and many tests use intentionally malformed HTML (e.g., <div>{{foo}}<div> with no closing tag)

        let options = TransformOptions::default();
        let transformer = HtmlToR3Transform::new(&allocator, input, options);
        let r3_result = transformer.transform(&html_result.nodes);

        let mut collector = ExpressionSourceCollector::new(
            input,
            extract_sub_expressions,
            find_implicit_receiver,
            template_expressions_only,
            use_unparser,
            unparse_full_interpolation,
        );
        visit_all(&mut collector, &r3_result.nodes);

        collector.sources
    }
}

impl Default for AstSpansRunner {
    fn default() -> Self {
        Self::new()
    }
}

impl SubsystemRunner for AstSpansRunner {
    fn name(&self) -> &'static str {
        "ast_spans"
    }

    fn description(&self) -> &'static str {
        "Angular AST span extraction (humanizeExpressionSource)"
    }

    fn can_handle(&self, assertion: &TestAssertion) -> bool {
        matches!(assertion, TestAssertion::HumanizeExpressionSource { .. })
    }

    fn run_assertion(&self, assertion: &TestAssertion) -> TestResult {
        match assertion {
            TestAssertion::HumanizeExpressionSource {
                input,
                expected,
                options,
                extract_sub_expressions,
                find_implicit_receiver,
                template_expressions_only,
                use_unparser,
                unparse_full_interpolation,
            } => {
                let actual = self.extract_expression_sources(
                    input,
                    options.as_ref(),
                    *extract_sub_expressions,
                    *find_implicit_receiver,
                    *template_expressions_only,
                    *use_unparser,
                    *unparse_full_interpolation,
                );

                // Convert expected JSON to comparable format
                let expected_strs: Vec<Vec<String>> = expected
                    .iter()
                    .filter_map(|v| {
                        if let serde_json::Value::Array(arr) = v {
                            Some(arr.iter().filter_map(|s| s.as_str().map(String::from)).collect())
                        } else {
                            None
                        }
                    })
                    .collect();

                if actual == expected_strs {
                    TestResult::Passed
                } else {
                    TestResult::Failed {
                        expected: format!("{expected_strs:?}"),
                        actual: format!("{actual:?}"),
                        diff: None,
                    }
                }
            }
            _ => TestResult::Skipped { reason: "Not handled by ast spans runner".to_string() },
        }
    }
}

/// Visitor to collect expression source strings from R3 AST
struct ExpressionSourceCollector<'a> {
    source: &'a str,
    sources: Vec<Vec<String>>,
    /// If true, extract sub-expressions instead of whole expressions
    extract_sub_expressions: bool,
    /// If true, recurse to find and output ImplicitReceiver spans
    find_implicit_receiver: bool,
    /// If true, only extract expressions from template attributes (structural directives)
    template_expressions_only: bool,
    /// If true, use unparser formatting instead of raw source
    use_unparser: bool,
    /// If true, output full interpolation `{{ expr }}`; if false, output inner expressions only
    unparse_full_interpolation: bool,
}

impl<'a> ExpressionSourceCollector<'a> {
    fn new(
        source: &'a str,
        extract_sub_expressions: bool,
        find_implicit_receiver: bool,
        template_expressions_only: bool,
        use_unparser: bool,
        unparse_full_interpolation: bool,
    ) -> Self {
        Self {
            source,
            sources: vec![],
            extract_sub_expressions,
            find_implicit_receiver,
            template_expressions_only,
            use_unparser,
            unparse_full_interpolation,
        }
    }

    /// Extract source text for an expression using its absolute source span
    fn extract_expression_source(&self, expr: &AngularExpression<'_>) -> Option<String> {
        let source_span = expr.source_span();
        let start = source_span.start as usize;
        let end = source_span.end as usize;
        if start <= end && end <= self.source.len() {
            Some(self.source[start..end].to_string())
        } else {
            None
        }
    }

    fn record_expression(&mut self, expr: &AngularExpression<'_>) {
        self.sources.push(vec![unparse_expression(expr)]);
    }

    fn record_source(&mut self, expr: &AngularExpression<'_>) {
        if let Some(source) = self.extract_expression_source(expr) {
            self.sources.push(vec![source]);
        }
    }

    // Event expressions are parsed from decoded attribute values, so their offsets
    // cannot be used to slice the original HTML when it contains character references.
    fn visit_event_handler(&mut self, handler: &AngularExpression<'_>) {
        if self.extract_sub_expressions {
            self.collect_sub_expressions(handler, Self::record_expression);
        } else {
            self.record_expression(handler);
        }
    }

    /// Collect sub-expressions from an expression based on its type
    fn collect_sub_expressions(
        &mut self,
        expr: &AngularExpression<'_>,
        record: fn(&mut Self, &AngularExpression<'_>),
    ) {
        match expr {
            // Binary: for assignments, only collect from right side; for others, both sides
            AngularExpression::Binary(bin) => {
                if bin.operation == BinaryOperator::Assign {
                    // For assignment expressions (property write), only collect the value (right side)
                    // unless left is a keyed expression which has interesting sub-expressions
                    if let AngularExpression::KeyedRead(_) = &bin.left {
                        self.collect_sub_expressions(&bin.left, record);
                    }
                    self.collect_sub_expressions(&bin.right, record);
                } else {
                    // For other binary operators, collect from both sides
                    self.collect_sub_expressions(&bin.left, record);
                    self.collect_sub_expressions(&bin.right, record);
                }
            }
            // Conditional: collect condition, true, and false expressions
            AngularExpression::Conditional(cond) => {
                record(self, &cond.condition);
                record(self, &cond.true_exp);
                record(self, &cond.false_exp);
            }
            // Chain: collect each expression in the chain
            AngularExpression::Chain(chain) => {
                for child in &chain.expressions {
                    record(self, child);
                }
            }
            // PropertyRead: output the receiver's source or recurse to find ImplicitReceiver
            AngularExpression::PropertyRead(prop) => {
                if self.find_implicit_receiver {
                    // Recurse to find ImplicitReceiver and output its (empty) span
                    self.collect_sub_expressions(&prop.receiver, record);
                } else {
                    if matches!(&prop.receiver, AngularExpression::ImplicitReceiver(_)) {
                        self.sources.push(vec![String::new()]);
                    } else {
                        record(self, &prop.receiver);
                    }
                }
            }
            // SafePropertyRead: output the receiver's source (for safe method call tests)
            AngularExpression::SafePropertyRead(prop) => {
                record(self, &prop.receiver);
            }
            // KeyedRead: collect the key
            AngularExpression::KeyedRead(keyed) => {
                record(self, &keyed.key);
            }
            // SafeKeyedRead: collect the key
            AngularExpression::SafeKeyedRead(keyed) => {
                record(self, &keyed.key);
            }
            // Call: collect from SafePropertyRead receiver (for safe method calls), then arguments
            AngularExpression::Call(call) => {
                // For safe method calls like prop?.safe(), collect from SafePropertyRead receiver
                if let AngularExpression::SafePropertyRead(safe_prop) = &call.receiver {
                    record(self, &safe_prop.receiver);
                }
                // Collect arguments
                for arg in &call.args {
                    record(self, arg);
                }
            }
            // SafeCall: collect receiver (for safe method calls)
            AngularExpression::SafeCall(call) => {
                // First collect the receiver
                record(self, &call.receiver);
                // Then collect arguments
                for arg in &call.args {
                    record(self, arg);
                }
            }
            // LiteralArray: collect elements
            AngularExpression::LiteralArray(arr) => {
                for elem in &arr.expressions {
                    record(self, elem);
                }
            }
            // LiteralMap: collect values
            AngularExpression::LiteralMap(map) => {
                for value in &map.values {
                    record(self, value);
                }
            }
            // PrefixNot: collect the operand
            AngularExpression::PrefixNot(not) => {
                record(self, &not.expression);
            }
            // Unary: collect the operand
            AngularExpression::Unary(unary) => {
                record(self, &unary.expr);
            }
            // NonNullAssert: collect the expression
            AngularExpression::NonNullAssert(nna) => {
                record(self, &nna.expression);
            }
            // BindingPipe: collect the input expression
            AngularExpression::BindingPipe(pipe) => {
                record(self, &pipe.exp);
            }
            // Interpolation: recursively collect from each expression
            AngularExpression::Interpolation(interp) => {
                for expr in &interp.expressions {
                    self.collect_sub_expressions(expr, record);
                }
            }
            // LiteralPrimitive: output the value (for binary operands, array elements, etc.)
            AngularExpression::LiteralPrimitive(_) => record(self, expr),
            // Empty expression or ImplicitReceiver: output empty string
            AngularExpression::Empty(_) | AngularExpression::ImplicitReceiver(_) => {
                self.sources.push(vec![String::new()]);
            }
            // SpreadElement: collect the expression being spread
            AngularExpression::SpreadElement(spread) => {
                record(self, &spread.expression);
            }
            // ArrowFunction: collect the body and parameter spans
            AngularExpression::ArrowFunction(arrow) => {
                // Collect the body expression
                record(self, &arrow.body);
            }
            // Terminal expressions - no sub-expressions to extract
            AngularExpression::ThisReceiver(_)
            | AngularExpression::TypeofExpression(_)
            | AngularExpression::VoidExpression(_)
            | AngularExpression::TaggedTemplateLiteral(_)
            | AngularExpression::TemplateLiteral(_)
            | AngularExpression::ParenthesizedExpression(_)
            | AngularExpression::RegularExpressionLiteral(_) => {}
        }
    }

    /// Process an expression - either extract the whole thing or its sub-expressions
    fn process_expression(&mut self, expr: &AngularExpression<'_>) {
        if self.extract_sub_expressions {
            self.collect_sub_expressions(expr, Self::record_source);
        } else if self.use_unparser {
            self.record_expression(expr);
        } else {
            self.record_source(expr);
        }
    }
}

impl<'b> R3Visitor<'b> for ExpressionSourceCollector<'b> {
    fn visit_bound_text(&mut self, text: &R3BoundText<'b>) {
        // Skip bound text when we only want template expressions
        if self.template_expressions_only {
            return;
        }
        // Handle interpolations
        if let AngularExpression::Interpolation(interp) = &text.value {
            if self.use_unparser && self.unparse_full_interpolation {
                // Output the entire interpolation with {{ }} formatting
                self.process_expression(&text.value);
            } else if self.use_unparser {
                // Unparse inner expressions only (for pipe tests)
                for expr in &interp.expressions {
                    let unparsed = unparse_expression(expr);
                    self.sources.push(vec![unparsed]);
                }
            } else {
                // Extract raw source of each expression
                for expr in &interp.expressions {
                    self.process_expression(expr);
                }
            }
        } else {
            self.process_expression(&text.value);
        }
    }

    fn visit_element(&mut self, element: &R3Element<'b>) {
        // Skip element inputs/outputs when we only want template expressions
        if !self.template_expressions_only {
            // Visit bound inputs (attributes)
            for attr in &element.inputs {
                if let AngularExpression::Interpolation(ref interp) = attr.value {
                    for expr in &interp.expressions {
                        self.process_expression(expr);
                    }
                } else if self.extract_sub_expressions {
                    self.collect_sub_expressions(&attr.value, Self::record_source);
                } else if let Some(ref value_span) = attr.value_span {
                    let start = value_span.start as usize;
                    let end = value_span.end as usize;
                    if end <= self.source.len() {
                        self.sources.push(vec![self.source[start..end].to_string()]);
                    }
                }
            }
            // Visit bound outputs (events)
            for event in &element.outputs {
                self.visit_event_handler(&event.handler);
            }
        }
        // Visit children
        visit_all(self, &element.children);
    }

    fn visit_template(&mut self, template: &R3Template<'b>) {
        if self.template_expressions_only {
            // Only visit template attributes (structural directive expressions like *ngFor)
            for attr in &template.template_attrs {
                if let R3TemplateAttr::Bound(bound_attr) = attr {
                    self.process_expression(&bound_attr.value);
                }
            }
        } else {
            // Visit bound inputs
            for attr in &template.inputs {
                if let AngularExpression::Interpolation(ref interp) = attr.value {
                    for expr in &interp.expressions {
                        self.process_expression(expr);
                    }
                } else if self.extract_sub_expressions {
                    self.collect_sub_expressions(&attr.value, Self::record_source);
                } else if let Some(ref value_span) = attr.value_span {
                    let start = value_span.start as usize;
                    let end = value_span.end as usize;
                    if end <= self.source.len() {
                        self.sources.push(vec![self.source[start..end].to_string()]);
                    }
                }
            }
            // Visit template attributes (from structural directive microsyntax like *ngIf)
            for attr in &template.template_attrs {
                if let R3TemplateAttr::Bound(bound_attr) = attr {
                    self.process_expression(&bound_attr.value);
                }
            }
            // Visit bound outputs
            for event in &template.outputs {
                self.visit_event_handler(&event.handler);
            }
            // Visit children
            visit_all(self, &template.children);
        }
    }

    fn visit_if_block(&mut self, block: &R3IfBlock<'b>) {
        for branch in &block.branches {
            if let Some(ref expr) = branch.expression {
                self.process_expression(expr);
            }
            visit_all(self, &branch.children);
        }
    }

    fn visit_boundary_error_block(&mut self, block: &R3BoundaryErrorBlock<'b>) {
        if let Some(ref expr) = block.expression {
            self.process_expression(expr);
        }
        visit_all(self, &block.children);
    }

    fn visit_for_loop_block(&mut self, block: &R3ForLoopBlock<'b>) {
        self.process_expression(&block.expression.ast);
        visit_all(self, &block.children);
        if let Some(ref empty) = block.empty {
            visit_all(self, &empty.children);
        }
    }

    fn visit_switch_block(&mut self, block: &R3SwitchBlock<'b>) {
        for group in &block.groups {
            visit_all(self, &group.children);
        }
    }

    fn visit_deferred_block(&mut self, block: &R3DeferredBlock<'b>) {
        visit_all(self, &block.children);
        if let Some(ref placeholder) = block.placeholder {
            visit_all(self, &placeholder.children);
        }
        if let Some(ref loading) = block.loading {
            visit_all(self, &loading.children);
        }
        if let Some(ref error) = block.error {
            visit_all(self, &error.children);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::AstSpansRunner;

    #[test]
    fn event_expression_text_uses_decoded_attribute_values() {
        for tag in ["button", "ng-template"] {
            let html = format!(r#"<{tag} (click)="save(&quot;x&quot;) // tail"></{tag}>"#);
            for (sub_expressions, expected) in [(false, r#"save("x")"#), (true, r#""x""#)] {
                let texts = AstSpansRunner::new().extract_expression_sources(
                    &html,
                    None,
                    sub_expressions,
                    false,
                    false,
                    false,
                    false,
                );
                assert_eq!(texts, [vec![expected]], "{html}");
            }
        }
    }
}

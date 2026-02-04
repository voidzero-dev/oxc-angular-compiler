//! JavaScript code emitter for Angular template compilation.
//!
//! This module emits JavaScript code from the Output AST after all
//! IR transformation phases are complete.
//!
//! Ported from Angular's `output/abstract_emitter.ts` and `output/abstract_js_emitter.ts`.
//!
//! ## Source Map Support
//!
//! The emitter supports source map generation via the `ParseSourceSpan` type.
//! Each emitted code part can be associated with a source span, which is used
//! to generate V3 source maps for debugging.
//!
//! See: `packages/compiler/src/output/abstract_emitter.ts:126-184`

use std::sync::Arc;

use oxc_diagnostics::OxcDiagnostic;
use oxc_span::{Atom, Span};

use super::ast::{
    ArrowFunctionBody, BinaryOperator, DeclareVarStmt, DynamicImportUrl, FnParam, LeadingComment,
    LiteralValue, OutputExpression, OutputStatement, UnaryOperator,
};
use crate::util::{ParseSourceFile, ParseSourceSpan};

// ============================================================================
// Constants
// ============================================================================

const INDENT_WITH: &str = "  ";
const LINE_LENGTH_LIMIT: usize = 80;

// ============================================================================
// Emitter Context
// ============================================================================

/// A single emitted line with source span tracking.
///
/// Tracks both the emitted code parts and their corresponding source spans.
/// See: `packages/compiler/src/output/abstract_emitter.ts:18-23`
#[derive(Debug)]
struct EmittedLine {
    /// Current indentation level.
    indent: usize,
    /// Parts of the line (code fragments).
    parts: Vec<String>,
    /// Source spans for each part (parallel array with `parts`).
    /// `None` indicates generated code without a source mapping.
    src_spans: Vec<Option<ParseSourceSpan>>,
    /// Total length of all parts.
    parts_length: usize,
}

impl EmittedLine {
    fn new(indent: usize) -> Self {
        Self { indent, parts: Vec::new(), src_spans: Vec::new(), parts_length: 0 }
    }
}

/// Context for emitting code, managing indentation and lines.
///
/// Tracks source spans for source map generation.
/// See: `packages/compiler/src/output/abstract_emitter.ts:45-126`
#[derive(Debug)]
pub struct EmitterContext {
    /// Current indentation level.
    indent: usize,
    /// All emitted lines.
    lines: Vec<EmittedLine>,
    /// Diagnostics collected during emission.
    pub diagnostics: std::vec::Vec<OxcDiagnostic>,
    /// Source file for source map generation.
    /// Stores the URL, content, and provides byte offset to line/column conversion.
    source_file: Option<Arc<ParseSourceFile>>,
}

impl EmitterContext {
    /// Create a new root context.
    pub fn new() -> Self {
        Self {
            indent: 0,
            lines: vec![EmittedLine::new(0)],
            diagnostics: std::vec::Vec::new(),
            source_file: None,
        }
    }

    /// Create a new context with source file information for source maps.
    ///
    /// The `ParseSourceFile` provides both the source URL and content,
    /// plus efficient byte offset to line/column conversion.
    pub fn with_source_file(source_file: Arc<ParseSourceFile>) -> Self {
        Self {
            indent: 0,
            lines: vec![EmittedLine::new(0)],
            diagnostics: std::vec::Vec::new(),
            source_file: Some(source_file),
        }
    }

    /// Convert an `oxc_span::Span` (byte offsets) to a `ParseSourceSpan` (line/column).
    ///
    /// Returns `None` if no source file is available.
    pub fn span_to_source_span(&self, span: Span) -> Option<ParseSourceSpan> {
        let file = self.source_file.as_ref()?;
        Some(ParseSourceSpan::from_offsets(file, span.start, span.end, None, None))
    }

    /// Get the current line.
    ///
    /// # Invariant
    /// Lines is always non-empty: initialized with one element in `new()`,
    /// and `println()` only pushes (never pops below 1).
    fn current_line(&self) -> &EmittedLine {
        debug_assert!(!self.lines.is_empty(), "lines should never be empty");
        &self.lines[self.lines.len() - 1]
    }

    /// Get the current line mutably.
    ///
    /// # Invariant
    /// Lines is always non-empty: initialized with one element in `new()`,
    /// and `println()` only pushes (never pops below 1).
    fn current_line_mut(&mut self) -> &mut EmittedLine {
        debug_assert!(!self.lines.is_empty(), "lines should never be empty");
        let len = self.lines.len();
        &mut self.lines[len - 1]
    }

    /// Print a string to the current line without source mapping.
    pub fn print(&mut self, part: &str) {
        self.print_with_span(part, None);
    }

    /// Print a string with an optional source span for source mapping.
    ///
    /// This is the core print method that tracks source spans.
    /// See: `packages/compiler/src/output/abstract_emitter.ts:89-98`
    pub fn print_with_span(&mut self, part: &str, source_span: Option<ParseSourceSpan>) {
        if !part.is_empty() {
            let line = self.current_line_mut();
            // Use chars().count() to count Unicode characters, not bytes.
            // This matches Angular's JavaScript behavior where "ɵɵ" counts as 2 chars.
            line.parts_length += part.chars().count();
            line.parts.push(part.to_string());
            line.src_spans.push(source_span);
        }
    }

    /// Print a string and start a new line.
    pub fn println(&mut self, part: &str) {
        self.println_with_span(part, None);
    }

    /// Print a string with source span and start a new line.
    pub fn println_with_span(&mut self, part: &str, source_span: Option<ParseSourceSpan>) {
        self.print_with_span(part, source_span);
        self.lines.push(EmittedLine::new(self.indent));
    }

    /// Start a new line.
    pub fn newline(&mut self) {
        self.lines.push(EmittedLine::new(self.indent));
    }

    /// Check if the current line is empty.
    pub fn line_is_empty(&self) -> bool {
        self.current_line().parts.is_empty()
    }

    /// Get the current line length including indentation.
    pub fn line_length(&self) -> usize {
        let line = self.current_line();
        line.indent * INDENT_WITH.len() + line.parts_length
    }

    /// Increase indentation.
    pub fn inc_indent(&mut self) {
        self.indent += 1;
        if self.line_is_empty() {
            self.current_line_mut().indent = self.indent;
        }
    }

    /// Decrease indentation.
    pub fn dec_indent(&mut self) {
        self.indent = self.indent.saturating_sub(1);
        if self.line_is_empty() {
            self.current_line_mut().indent = self.indent;
        }
    }

    /// Remove the last line if it's empty.
    pub fn remove_empty_last_line(&mut self) {
        if self.line_is_empty() && self.lines.len() > 1 {
            self.lines.pop();
        }
    }

    /// Convert the context to a source string.
    pub fn to_source(&self) -> String {
        let source_lines: &[EmittedLine] =
            if !self.lines.is_empty() && self.lines.last().is_some_and(|l| l.parts.is_empty()) {
                &self.lines[..self.lines.len() - 1]
            } else {
                &self.lines
            };

        source_lines
            .iter()
            .map(|line| {
                if line.parts.is_empty() {
                    String::new()
                } else {
                    let indent = INDENT_WITH.repeat(line.indent);
                    format!("{}{}", indent, line.parts.join(""))
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Generate a source map from the accumulated source spans.
    ///
    /// Returns `None` if no source file information was provided or no mappings exist.
    ///
    /// See: `packages/compiler/src/output/abstract_emitter.ts:126-184`
    pub fn to_source_map(&self, generated_file: Option<&str>) -> Option<oxc_sourcemap::SourceMap> {
        // Need source file to generate a source map
        let source_file = self.source_file.as_ref()?;
        let source_url = &source_file.url;
        let source_content = Some(source_file.content.as_ref());

        let mut builder = oxc_sourcemap::SourceMapBuilder::default();

        // Set the generated file name if provided
        if let Some(file) = generated_file {
            builder.set_file(file);
        }

        // Register the source file with content
        let source_id =
            builder.set_source_and_content(source_url.as_ref(), source_content.unwrap_or(" "));

        let source_lines: &[EmittedLine] =
            if !self.lines.is_empty() && self.lines.last().is_some_and(|l| l.parts.is_empty()) {
                &self.lines[..self.lines.len() - 1]
            } else {
                &self.lines
            };

        let mut has_mappings = false;

        // Iterate through each line
        for (generated_line, line) in source_lines.iter().enumerate() {
            // Calculate column position within the generated line
            let indent_len = line.indent * INDENT_WITH.len();
            let mut generated_col = indent_len;

            // Track which spans we've seen to deduplicate consecutive mappings
            let mut last_span: Option<&ParseSourceSpan> = None;

            for (part_idx, part) in line.parts.iter().enumerate() {
                let src_span = line.src_spans.get(part_idx).and_then(|s| s.as_ref());

                // Only emit a mapping if we have a span and it's different from the last one
                if let Some(span) = src_span {
                    let should_emit = match last_span {
                        None => true,
                        Some(last) => {
                            span.start.offset != last.start.offset
                                || span.start.line != last.start.line
                                || span.start.col != last.start.col
                        }
                    };

                    if should_emit {
                        #[expect(clippy::cast_possible_truncation)]
                        builder.add_token(
                            generated_line as u32,
                            generated_col as u32,
                            span.start.line,
                            span.start.col,
                            Some(source_id),
                            None,
                        );
                        has_mappings = true;
                        last_span = Some(span);
                    }
                }

                generated_col += part.len();
            }
        }

        if has_mappings { Some(builder.into_sourcemap()) } else { None }
    }

    /// Generate source and source map together.
    ///
    /// Returns a tuple of (source_code, source_map).
    /// The source map may be `None` if no source file information was provided.
    pub fn to_source_with_map(
        &self,
        generated_file: Option<&str>,
    ) -> (String, Option<oxc_sourcemap::SourceMap>) {
        (self.to_source(), self.to_source_map(generated_file))
    }
}

impl Default for EmitterContext {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// JavaScript Emitter
// ============================================================================

// ============================================================================
// Source Span Helpers
// ============================================================================

/// Get the source span from an `OutputExpression`.
fn get_source_span(expr: &OutputExpression<'_>) -> Option<Span> {
    match expr {
        OutputExpression::Literal(e) => e.source_span,
        OutputExpression::LiteralArray(e) => e.source_span,
        OutputExpression::LiteralMap(e) => e.source_span,
        OutputExpression::RegularExpressionLiteral(e) => e.source_span,
        OutputExpression::TemplateLiteral(e) => e.source_span,
        OutputExpression::TaggedTemplateLiteral(e) => e.source_span,
        OutputExpression::ReadVar(e) => e.source_span,
        OutputExpression::ReadProp(e) => e.source_span,
        OutputExpression::ReadKey(e) => e.source_span,
        OutputExpression::BinaryOperator(e) => e.source_span,
        OutputExpression::UnaryOperator(e) => e.source_span,
        OutputExpression::Conditional(e) => e.source_span,
        OutputExpression::Not(e) => e.source_span,
        OutputExpression::Typeof(e) => e.source_span,
        OutputExpression::Void(e) => e.source_span,
        OutputExpression::Parenthesized(e) => e.source_span,
        OutputExpression::Comma(e) => e.source_span,
        OutputExpression::Function(e) => e.source_span,
        OutputExpression::ArrowFunction(e) => e.source_span,
        OutputExpression::InvokeFunction(e) => e.source_span,
        OutputExpression::Instantiate(e) => e.source_span,
        OutputExpression::DynamicImport(e) => e.source_span,
        OutputExpression::External(e) => e.source_span,
        OutputExpression::LocalizedString(e) => e.source_span,
        OutputExpression::WrappedNode(e) => e.source_span,
        OutputExpression::WrappedIrNode(e) => e.source_span,
        OutputExpression::SpreadElement(e) => e.source_span,
    }
}

// ============================================================================
// JavaScript Emitter
// ============================================================================

/// JavaScript code emitter.
///
/// Converts Output AST to JavaScript source code.
pub struct JsEmitter {
    /// Whether to escape $ in strings.
    escape_dollar_in_strings: bool,
}

impl JsEmitter {
    /// Create a new JavaScript emitter.
    pub fn new() -> Self {
        Self { escape_dollar_in_strings: false }
    }

    /// Emit an expression to a string.
    pub fn emit_expression<'a>(&self, expr: &OutputExpression<'a>) -> String {
        let mut ctx = EmitterContext::new();
        self.visit_expression(expr, &mut ctx);
        ctx.to_source()
    }

    /// Emit a statement to a string.
    pub fn emit_statement<'a>(&self, stmt: &OutputStatement<'a>) -> String {
        let mut ctx = EmitterContext::new();
        self.visit_statement(stmt, &mut ctx);
        ctx.to_source()
    }

    /// Emit multiple statements to a string.
    pub fn emit_statements<'a>(&self, stmts: &[OutputStatement<'a>]) -> String {
        let mut ctx = EmitterContext::new();
        self.visit_all_statements(stmts, &mut ctx);
        ctx.to_source()
    }

    /// Emit an expression with source map support.
    ///
    /// Returns a tuple of (source_code, source_map).
    /// The source map will be present if source spans are available in the expression.
    pub fn emit_expression_with_source_map<'a>(
        &self,
        expr: &OutputExpression<'a>,
        source_file: Arc<ParseSourceFile>,
        generated_file: Option<&str>,
    ) -> (String, Option<oxc_sourcemap::SourceMap>) {
        let mut ctx = EmitterContext::with_source_file(source_file);
        self.visit_expression(expr, &mut ctx);
        ctx.to_source_with_map(generated_file)
    }

    /// Emit multiple statements with source map support.
    ///
    /// Returns a tuple of (source_code, source_map).
    /// The source map will be present if source spans are available in the statements.
    pub fn emit_statements_with_source_map<'a>(
        &self,
        stmts: &[OutputStatement<'a>],
        source_file: Arc<ParseSourceFile>,
        generated_file: Option<&str>,
    ) -> (String, Option<oxc_sourcemap::SourceMap>) {
        let mut ctx = EmitterContext::with_source_file(source_file);
        self.visit_all_statements(stmts, &mut ctx);
        ctx.to_source_with_map(generated_file)
    }

    // ========================================================================
    // Statement Visitors
    // ========================================================================

    fn visit_statement<'a>(&self, stmt: &OutputStatement<'a>, ctx: &mut EmitterContext) {
        match stmt {
            OutputStatement::DeclareVar(s) => self.visit_declare_var_stmt(s, ctx),
            OutputStatement::DeclareFunction(s) => self.visit_declare_function_stmt(s, ctx),
            OutputStatement::Expression(s) => self.visit_expression_stmt(&s.expr, ctx),
            OutputStatement::Return(s) => self.visit_return_stmt(&s.value, ctx),
            OutputStatement::If(s) => self.visit_if_stmt(s, ctx),
        }
    }

    /// Visit all statements, adding blank lines between top-level const/function declarations.
    ///
    /// This matches Angular's output formatting which adds a blank line between:
    /// - Consecutive const declarations
    /// - Consecutive function declarations
    /// - Between const and function declarations
    fn visit_all_statements<'a>(&self, stmts: &[OutputStatement<'a>], ctx: &mut EmitterContext) {
        for (i, stmt) in stmts.iter().enumerate() {
            // Add blank line between top-level declarations (const/function)
            if i > 0 {
                let prev_stmt = &stmts[i - 1];
                let needs_blank_line = matches!(
                    (prev_stmt, stmt),
                    (OutputStatement::DeclareVar(_), OutputStatement::DeclareVar(_))
                        | (OutputStatement::DeclareVar(_), OutputStatement::DeclareFunction(_))
                        | (
                            OutputStatement::DeclareFunction(_),
                            OutputStatement::DeclareFunction(_)
                        )
                        | (OutputStatement::DeclareFunction(_), OutputStatement::DeclareVar(_))
                );
                if needs_blank_line {
                    ctx.newline();
                }
            }
            self.visit_statement(stmt, ctx);
        }
    }

    fn visit_declare_var_stmt(&self, stmt: &DeclareVarStmt<'_>, ctx: &mut EmitterContext) {
        // Print leading comment if present
        // See: packages/compiler/src/output/abstract_emitter.ts:218-235
        if let Some(ref comment) = stmt.leading_comment {
            self.print_leading_comment(comment, ctx);
        }

        // Use 'const' for FINAL modifier, 'let' otherwise
        // This matches Angular's TypeScript translator behavior:
        // See: packages/compiler-cli/src/ngtsc/translator/src/translator.ts:87-101
        // Angular's abstract_js_emitter always uses 'var' for downleveled output,
        // but the TypeScript translator uses const/let for modern output.
        let keyword =
            if stmt.modifiers.has(super::ast::StmtModifier::FINAL) { "const " } else { "let " };
        ctx.print(keyword);
        ctx.print(&stmt.name);
        if let Some(ref value) = stmt.value {
            ctx.print(" = ");
            self.visit_expression(value, ctx);
        }
        ctx.println(";");
    }

    /// Print a leading comment.
    ///
    /// See: `packages/compiler/src/output/abstract_emitter.ts:218-235`
    fn print_leading_comment(&self, comment: &LeadingComment<'_>, ctx: &mut EmitterContext) {
        match comment {
            LeadingComment::JSDoc(jsdoc) => {
                // Format JSDoc comment with @desc, @meaning, and @suppress tags
                // See: packages/compiler/src/output/output_jit_trusted_types.ts
                ctx.print("/**");
                if let Some(ref desc) = jsdoc.description {
                    ctx.print(" @desc ");
                    ctx.print(desc.as_str());
                }
                if let Some(ref meaning) = jsdoc.meaning {
                    ctx.print(" @meaning ");
                    ctx.print(meaning.as_str());
                }
                if jsdoc.suppress_msg_descriptions {
                    ctx.print(" @suppress {msgDescriptions}");
                }
                ctx.println(" */");
            }
            LeadingComment::SingleLine(text) => {
                ctx.print("// ");
                ctx.println(text.as_str());
            }
            LeadingComment::MultiLine(text) => {
                // Format multi-line comments with proper leading space before * on continuation lines.
                // Angular outputs:
                //   /*
                //    * @license
                //    * Copyright Google LLC
                //    */
                // Note the space before * on each line.
                let formatted = text
                    .as_str()
                    .lines()
                    .enumerate()
                    .map(|(i, line)| {
                        if i == 0 {
                            // First line: just the content (will be prefixed with /*)
                            line.to_string()
                        } else {
                            // Continuation lines: ensure leading space before *
                            let trimmed = line.trim_start();
                            if trimmed.starts_with('*') {
                                format!(" {trimmed}")
                            } else {
                                format!(" * {line}")
                            }
                        }
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                ctx.print("/*");
                ctx.print(&formatted);
                ctx.println(" */");
            }
        }
    }

    fn visit_declare_function_stmt(
        &self,
        stmt: &super::ast::DeclareFunctionStmt<'_>,
        ctx: &mut EmitterContext,
    ) {
        ctx.print("function ");
        ctx.print(&stmt.name);
        ctx.print("(");
        self.visit_params(&stmt.params, ctx);
        ctx.println(") {");
        ctx.inc_indent();
        for s in &stmt.statements {
            self.visit_statement(s, ctx);
        }
        ctx.dec_indent();
        ctx.println("}");
    }

    fn visit_expression_stmt<'a>(&self, expr: &OutputExpression<'a>, ctx: &mut EmitterContext) {
        self.visit_expression(expr, ctx);
        ctx.println(";");
    }

    fn visit_return_stmt<'a>(&self, value: &OutputExpression<'a>, ctx: &mut EmitterContext) {
        ctx.print("return ");
        self.visit_expression(value, ctx);
        ctx.println(";");
    }

    fn visit_if_stmt(&self, stmt: &super::ast::IfStmt<'_>, ctx: &mut EmitterContext) {
        ctx.print("if (");
        self.visit_expression(&stmt.condition, ctx);
        ctx.print(") {");

        let has_else = !stmt.false_case.is_empty();
        if stmt.true_case.len() <= 1 && !has_else {
            ctx.print(" ");
            for s in &stmt.true_case {
                self.visit_statement(s, ctx);
            }
            ctx.remove_empty_last_line();
            ctx.print(" ");
        } else {
            ctx.newline();
            ctx.inc_indent();
            for s in &stmt.true_case {
                self.visit_statement(s, ctx);
            }
            ctx.dec_indent();
            if has_else {
                ctx.println("} else {");
                ctx.inc_indent();
                for s in &stmt.false_case {
                    self.visit_statement(s, ctx);
                }
                ctx.dec_indent();
            }
        }
        ctx.println("}");
    }

    // ========================================================================
    // Expression Visitors
    // ========================================================================

    fn visit_expression<'a>(&self, expr: &OutputExpression<'a>, ctx: &mut EmitterContext) {
        // Get the source span for this expression (if available)
        let source_span = get_source_span(expr).and_then(|span| ctx.span_to_source_span(span));

        match expr {
            OutputExpression::Literal(e) => self.visit_literal(&e.value, source_span, ctx),
            OutputExpression::LiteralArray(e) => self.visit_literal_array(&e.entries, ctx),
            OutputExpression::LiteralMap(e) => self.visit_literal_map(&e.entries, ctx),
            OutputExpression::RegularExpressionLiteral(e) => {
                ctx.print_with_span("/", source_span);
                ctx.print(&e.body);
                ctx.print("/");
                if let Some(ref flags) = e.flags {
                    ctx.print(flags);
                }
            }
            OutputExpression::TemplateLiteral(e) => self.visit_template_literal(e, ctx),
            OutputExpression::TaggedTemplateLiteral(e) => {
                self.visit_tagged_template_literal(e, ctx);
            }
            OutputExpression::ReadVar(e) => {
                // Variable references are key for source mapping - map the variable name
                let var_span = e.source_span.and_then(|span| ctx.span_to_source_span(span));
                ctx.print_with_span(&e.name, var_span);
            }
            OutputExpression::ReadProp(e) => {
                self.visit_expression(&e.receiver, ctx);
                if e.optional {
                    ctx.print("?.");
                } else {
                    ctx.print(".");
                }
                // Map the property name to its source location
                let prop_span = e.source_span.and_then(|span| ctx.span_to_source_span(span));
                ctx.print_with_span(&e.name, prop_span);
            }
            OutputExpression::ReadKey(e) => {
                self.visit_expression(&e.receiver, ctx);
                if e.optional {
                    ctx.print("?.[");
                } else {
                    ctx.print("[");
                }
                self.visit_expression(&e.index, ctx);
                ctx.print("]");
            }
            OutputExpression::BinaryOperator(e) => {
                ctx.print("(");
                // Parentheses are required when mixing nullish coalescing (??) with logical
                // operators (&&, ||) to avoid JavaScript syntax errors.
                //
                // Required cases:
                // 1. `(a && b) ?? c` or `(a || b) ?? c` - logical on left of ??
                // 2. `a ?? (b && c)` or `a ?? (b || c)` - logical on right of ??
                // 3. `(a ?? b) && c` or `(a ?? b) || c` - ?? on left of logical
                // 4. `(a ? b : c) ?? d` - conditional on left of ??
                //
                // See: angular/packages/compiler/src/template/pipeline/src/phases/strip_nonrequired_parentheses.ts
                let lhs_needs_extra_parens = self.needs_extra_parens_for_lhs(e.operator, &e.lhs);
                let rhs_needs_extra_parens = self.needs_extra_parens_for_rhs(e.operator, &e.rhs);

                if lhs_needs_extra_parens {
                    ctx.print("(");
                }
                self.visit_expression(&e.lhs, ctx);
                if lhs_needs_extra_parens {
                    ctx.print(")");
                }
                ctx.print(" ");
                ctx.print(binary_operator_to_str(e.operator));
                ctx.print(" ");
                if rhs_needs_extra_parens {
                    ctx.print("(");
                }
                self.visit_expression(&e.rhs, ctx);
                if rhs_needs_extra_parens {
                    ctx.print(")");
                }
                ctx.print(")");
            }
            OutputExpression::UnaryOperator(e) => {
                if e.parens {
                    ctx.print("(");
                }
                ctx.print_with_span(unary_operator_to_str(e.operator), source_span);
                self.visit_expression(&e.expr, ctx);
                if e.parens {
                    ctx.print(")");
                }
            }
            OutputExpression::Conditional(e) => {
                ctx.print("(");
                self.visit_expression(&e.condition, ctx);
                ctx.print("? ");
                self.visit_expression(&e.true_case, ctx);
                ctx.print(": ");
                if let Some(ref false_case) = e.false_case {
                    self.visit_expression(false_case, ctx);
                } else {
                    ctx.print("null");
                }
                ctx.print(")");
            }
            OutputExpression::Not(e) => {
                ctx.print_with_span("!", source_span);
                self.visit_expression(&e.condition, ctx);
            }
            OutputExpression::Typeof(e) => {
                ctx.print_with_span("typeof ", source_span);
                self.visit_expression(&e.expr, ctx);
            }
            OutputExpression::Void(e) => {
                ctx.print_with_span("void ", source_span);
                self.visit_expression(&e.expr, ctx);
            }
            OutputExpression::Parenthesized(e) => {
                self.visit_expression(&e.expr, ctx);
            }
            OutputExpression::Comma(e) => {
                ctx.print("(");
                self.visit_all_expressions(&e.parts, ctx, ",");
                ctx.print(")");
            }
            OutputExpression::Function(e) => {
                ctx.print_with_span("function", source_span);
                if let Some(ref name) = e.name {
                    ctx.print(" ");
                    ctx.print(name);
                }
                ctx.print("(");
                self.visit_params(&e.params, ctx);
                ctx.println(") {");
                ctx.inc_indent();
                for s in &e.statements {
                    self.visit_statement(s, ctx);
                }
                ctx.dec_indent();
                ctx.print("}");
            }
            OutputExpression::ArrowFunction(e) => {
                ctx.print_with_span("(", source_span);
                self.visit_params(&e.params, ctx);
                ctx.print(") =>");
                match &e.body {
                    ArrowFunctionBody::Expression(body_expr) => {
                        // Check if the body is an object literal (needs parens)
                        let is_object_literal =
                            matches!(body_expr.as_ref(), OutputExpression::LiteralMap(_));
                        if is_object_literal {
                            ctx.print("(");
                        }
                        self.visit_expression(body_expr, ctx);
                        if is_object_literal {
                            ctx.print(")");
                        }
                    }
                    ArrowFunctionBody::Statements(stmts) => {
                        ctx.println("{");
                        ctx.inc_indent();
                        for s in stmts {
                            self.visit_statement(s, ctx);
                        }
                        ctx.dec_indent();
                        ctx.print("}");
                    }
                }
            }
            OutputExpression::InvokeFunction(e) => {
                // Emit /*@__PURE__*/ annotation if this is a pure call
                if e.pure {
                    ctx.print("/*@__PURE__*/ ");
                }
                // Wrap arrow functions in parens for IIFE
                let should_parenthesize =
                    matches!(e.fn_expr.as_ref(), OutputExpression::ArrowFunction(_));
                if should_parenthesize {
                    ctx.print("(");
                }
                self.visit_expression(&e.fn_expr, ctx);
                if should_parenthesize {
                    ctx.print(")");
                }
                // Map the function call to its source location
                // Use optional chaining syntax if this is an optional call
                if e.optional {
                    ctx.print_with_span("?.(", source_span);
                } else {
                    ctx.print_with_span("(", source_span);
                }
                self.visit_all_expressions(&e.args, ctx, ",");
                ctx.print(")");
            }
            OutputExpression::Instantiate(e) => {
                ctx.print_with_span("new ", source_span);
                self.visit_expression(&e.class_expr, ctx);
                ctx.print("(");
                self.visit_all_expressions(&e.args, ctx, ",");
                ctx.print(")");
            }
            OutputExpression::DynamicImport(e) => {
                ctx.print("import(");
                // Emit url_comment if present (e.g., /* @vite-ignore */)
                if let Some(comment) = &e.url_comment {
                    ctx.print("/* ");
                    ctx.print(comment);
                    ctx.print(" */ ");
                }
                match &e.url {
                    DynamicImportUrl::String(s) => {
                        ctx.print(&escape_string(s, self.escape_dollar_in_strings));
                    }
                    DynamicImportUrl::Expression(expr) => {
                        self.visit_expression(expr, ctx);
                    }
                }
                ctx.print(")");
            }
            OutputExpression::External(e) => {
                // External references are handled by the module system
                if let Some(ref name) = e.value.name {
                    ctx.print(name);
                }
            }
            OutputExpression::LocalizedString(e) => {
                self.visit_localized_string(e, ctx);
            }
            OutputExpression::WrappedNode(_) => {
                // Wrapped nodes should not appear in JavaScript output.
                // This matches Angular's abstract_js_emitter.ts which throws:
                // `throw new Error("Cannot emit a WrappedNodeExpr in Javascript.");`
                // WrappedNodeExpr is used internally during compilation but should
                // be resolved before emission - if we hit this, it's a compiler bug.
                ctx.diagnostics.push(OxcDiagnostic::error(
                    "Cannot emit a WrappedNodeExpr in JavaScript. WrappedNodeExpr should be resolved before emission."
                ));
                // Emit undefined as fallback
                ctx.print("undefined");
            }
            OutputExpression::WrappedIrNode(_) => {
                // Wrapped IR expressions should not appear in JavaScript output.
                // They should be resolved during the reify phase before emission.
                ctx.diagnostics.push(OxcDiagnostic::error(
                    "Cannot emit a WrappedIrExpr in JavaScript. WrappedIrExpr should be resolved before emission."
                ));
                // Emit undefined as fallback
                ctx.print("undefined");
            }
            OutputExpression::SpreadElement(e) => {
                ctx.print("...");
                self.visit_expression(&e.expr, ctx);
            }
        }
    }

    fn visit_literal(
        &self,
        value: &LiteralValue<'_>,
        source_span: Option<ParseSourceSpan>,
        ctx: &mut EmitterContext,
    ) {
        match value {
            LiteralValue::Null => ctx.print_with_span("null", source_span),
            LiteralValue::Undefined => ctx.print_with_span("undefined", source_span),
            LiteralValue::Boolean(b) => {
                ctx.print_with_span(if *b { "true" } else { "false" }, source_span);
            }
            LiteralValue::Number(n) => {
                // Use JS-compatible formatting to match Angular's template literal coercion
                ctx.print_with_span(&format_number_like_js(*n), source_span);
            }
            LiteralValue::String(s) => {
                ctx.print_with_span(&escape_string(s, self.escape_dollar_in_strings), source_span);
            }
        }
    }

    /// Visit a literal array expression.
    ///
    /// When the array would exceed the line length limit, formats it as multi-line
    /// with each element on its own line and a trailing comma after the last element.
    /// This matches the TypeScript printer behavior used by Angular's ngtsc compiler.
    ///
    /// Single-line: `[a,b,c]`
    /// Multi-line:
    /// ```text
    /// [element1,element2,element3,
    ///     element4,
    ///     element5]
    /// ```
    fn visit_literal_array<'a>(&self, entries: &[OutputExpression<'a>], ctx: &mut EmitterContext) {
        ctx.print("[");
        self.visit_all_expressions(entries, ctx, ",");
        ctx.print("]");
    }

    /// Visit a literal map (object literal) expression.
    ///
    /// Handles line-breaking for large objects - when line length exceeds 80 chars,
    /// entries are wrapped to new lines with double indent for continuations.
    ///
    /// See: `packages/compiler/src/output/abstract_emitter.ts:455-471` (visitLiteralMapExpr)
    /// See: `packages/compiler/src/output/abstract_emitter.ts:492-520` (visitAllObjects)
    fn visit_literal_map<'a>(
        &self,
        entries: &[super::ast::LiteralMapEntry<'a>],
        ctx: &mut EmitterContext,
    ) {
        ctx.print("{");
        let mut incremented_indent = false;
        for (i, entry) in entries.iter().enumerate() {
            if i > 0 {
                // Check line length and break if needed
                if ctx.line_length() > LINE_LENGTH_LIMIT {
                    ctx.println(",");
                    if !incremented_indent {
                        // Continuation lines are marked with double indent
                        ctx.inc_indent();
                        ctx.inc_indent();
                        incremented_indent = true;
                    }
                } else {
                    ctx.print(",");
                }
            }
            let key = escape_identifier(&entry.key, self.escape_dollar_in_strings, entry.quoted);
            ctx.print(&key);
            ctx.print(":");
            self.visit_expression(&entry.value, ctx);
        }
        if incremented_indent {
            ctx.dec_indent();
            ctx.dec_indent();
        }
        ctx.print("}");
    }

    fn visit_template_literal(
        &self,
        expr: &super::ast::TemplateLiteralExpr<'_>,
        ctx: &mut EmitterContext,
    ) {
        ctx.print("`");
        for (i, element) in expr.elements.iter().enumerate() {
            ctx.print(&element.raw_text);
            if i < expr.expressions.len() {
                ctx.print("${");
                self.visit_expression(&expr.expressions[i], ctx);
                ctx.print("}");
            }
        }
        ctx.print("`");
    }

    fn visit_tagged_template_literal(
        &self,
        expr: &super::ast::TaggedTemplateLiteralExpr<'_>,
        ctx: &mut EmitterContext,
    ) {
        // Downlevel tagged template to function call for compatibility
        // tag`...` becomes tag(__makeTemplateObject(cooked, raw), expr1, expr2, ...)
        const MAKE_TEMPLATE_OBJECT_POLYFILL: &str = "(this&&this.__makeTemplateObject||function(e,t){return Object.defineProperty?Object.defineProperty(e,\"raw\",{value:t}):e.raw=t,e})";

        self.visit_expression(&expr.tag, ctx);
        ctx.print("(");
        ctx.print(MAKE_TEMPLATE_OBJECT_POLYFILL);
        ctx.print("(");

        // Cooked strings
        ctx.print("[");
        let elements = &expr.template.elements;
        for (i, element) in elements.iter().enumerate() {
            if i > 0 {
                ctx.print(", ");
            }
            ctx.print(&escape_string(&element.text, false));
        }
        ctx.print("], ");

        // Raw strings
        ctx.print("[");
        for (i, element) in elements.iter().enumerate() {
            if i > 0 {
                ctx.print(", ");
            }
            ctx.print(&escape_string(&element.raw_text, false));
        }
        ctx.print("])");

        // Expressions
        for expression in &expr.template.expressions {
            ctx.print(", ");
            self.visit_expression(expression, ctx);
        }
        ctx.print(")");
    }

    fn visit_localized_string(
        &self,
        expr: &super::ast::LocalizedStringExpr<'_>,
        ctx: &mut EmitterContext,
    ) {
        // $localize`...` becomes $localize(__makeTemplateObject(cooked, raw), expr1, expr2, ...)
        const MAKE_TEMPLATE_OBJECT_POLYFILL: &str = "(this&&this.__makeTemplateObject||function(e,t){return Object.defineProperty?Object.defineProperty(e,\"raw\",{value:t}):e.raw=t,e})";

        ctx.print("$localize(");
        ctx.print(MAKE_TEMPLATE_OBJECT_POLYFILL);
        ctx.print("(");

        // Cooked strings (message parts)
        ctx.print("[");
        for (i, part) in expr.message_parts.iter().enumerate() {
            if i > 0 {
                ctx.print(", ");
            }
            ctx.print(&escape_string(part, false));
        }
        ctx.print("], ");

        // Raw strings (same as cooked for i18n)
        ctx.print("[");
        for (i, part) in expr.message_parts.iter().enumerate() {
            if i > 0 {
                ctx.print(", ");
            }
            ctx.print(&escape_string(part, false));
        }
        ctx.print("])");

        // Expressions
        for expression in &expr.expressions {
            ctx.print(", ");
            self.visit_expression(expression, ctx);
        }
        ctx.print(")");
    }

    fn visit_params(&self, params: &[FnParam<'_>], ctx: &mut EmitterContext) {
        let mut incremented_indent = false;
        for (i, param) in params.iter().enumerate() {
            if i > 0 {
                if ctx.line_length() > LINE_LENGTH_LIMIT {
                    ctx.println(",");
                    if !incremented_indent {
                        ctx.inc_indent();
                        ctx.inc_indent();
                        incremented_indent = true;
                    }
                } else {
                    ctx.print(",");
                }
            }
            ctx.print(&param.name);
        }
        if incremented_indent {
            ctx.dec_indent();
            ctx.dec_indent();
        }
    }

    fn visit_all_expressions<'a>(
        &self,
        expressions: &[OutputExpression<'a>],
        ctx: &mut EmitterContext,
        separator: &str,
    ) {
        let mut incremented_indent = false;
        for (i, expr) in expressions.iter().enumerate() {
            if i > 0 {
                if ctx.line_length() > LINE_LENGTH_LIMIT {
                    ctx.println(separator);
                    if !incremented_indent {
                        // Continuation lines are marked with double indent
                        ctx.inc_indent();
                        ctx.inc_indent();
                        incremented_indent = true;
                    }
                } else {
                    ctx.print(separator);
                }
            }
            self.visit_expression(expr, ctx);
        }
        if incremented_indent {
            ctx.dec_indent();
            ctx.dec_indent();
        }
    }

    // ========================================================================
    // Parentheses Helpers
    // ========================================================================

    /// Check if the LHS of a binary operator needs extra parentheses.
    ///
    /// Required when:
    /// - `??` operator with logical AND/OR or conditional on the left
    /// - `&&`/`||` operator with `??` on the left
    fn needs_extra_parens_for_lhs(&self, op: BinaryOperator, lhs: &OutputExpression<'_>) -> bool {
        match op {
            BinaryOperator::NullishCoalesce => {
                // (a && b) ?? c, (a || b) ?? c, (a ? b : c) ?? d
                matches!(lhs, OutputExpression::Conditional(_)) || is_logical_and_or(lhs)
            }
            BinaryOperator::And | BinaryOperator::Or => {
                // (a ?? b) && c, (a ?? b) || c
                is_nullish_coalesce(lhs)
            }
            _ => false,
        }
    }

    /// Check if the RHS of a binary operator needs extra parentheses.
    ///
    /// Required when:
    /// - `??` operator with logical AND/OR or conditional on the right
    fn needs_extra_parens_for_rhs(&self, op: BinaryOperator, rhs: &OutputExpression<'_>) -> bool {
        match op {
            BinaryOperator::NullishCoalesce => {
                // a ?? (b && c), a ?? (b || c), a ?? (b ? c : d)
                matches!(rhs, OutputExpression::Conditional(_)) || is_logical_and_or(rhs)
            }
            _ => false,
        }
    }
}

impl Default for JsEmitter {
    fn default() -> Self {
        Self::new()
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Convert a binary operator to its JavaScript string representation.
fn binary_operator_to_str(op: BinaryOperator) -> &'static str {
    match op {
        BinaryOperator::Equals => "==",
        BinaryOperator::NotEquals => "!=",
        BinaryOperator::Assign => "=",
        BinaryOperator::Identical => "===",
        BinaryOperator::NotIdentical => "!==",
        BinaryOperator::Minus => "-",
        BinaryOperator::Plus => "+",
        BinaryOperator::Divide => "/",
        BinaryOperator::Multiply => "*",
        BinaryOperator::Modulo => "%",
        BinaryOperator::And => "&&",
        BinaryOperator::Or => "||",
        BinaryOperator::BitwiseOr => "|",
        BinaryOperator::BitwiseAnd => "&",
        BinaryOperator::BitwiseXor => "^",
        BinaryOperator::LeftShift => "<<",
        BinaryOperator::RightShift => ">>",
        BinaryOperator::UnsignedRightShift => ">>>",
        BinaryOperator::Lower => "<",
        BinaryOperator::LowerEquals => "<=",
        BinaryOperator::Bigger => ">",
        BinaryOperator::BiggerEquals => ">=",
        BinaryOperator::NullishCoalesce => "??",
        BinaryOperator::Exponentiation => "**",
        BinaryOperator::In => "in",
        BinaryOperator::Instanceof => "instanceof",
        BinaryOperator::AdditionAssignment => "+=",
        BinaryOperator::SubtractionAssignment => "-=",
        BinaryOperator::MultiplicationAssignment => "*=",
        BinaryOperator::DivisionAssignment => "/=",
        BinaryOperator::RemainderAssignment => "%=",
        BinaryOperator::ExponentiationAssignment => "**=",
        BinaryOperator::AndAssignment => "&&=",
        BinaryOperator::OrAssignment => "||=",
        BinaryOperator::NullishCoalesceAssignment => "??=",
    }
}

/// Convert a unary operator to its JavaScript string representation.
fn unary_operator_to_str(op: UnaryOperator) -> &'static str {
    match op {
        UnaryOperator::Minus => "-",
        UnaryOperator::Plus => "+",
    }
}

/// Check if an expression is a logical AND or OR binary operation.
///
/// Used to detect cases where parentheses are required when mixing with `??`.
fn is_logical_and_or(expr: &OutputExpression<'_>) -> bool {
    if let OutputExpression::BinaryOperator(bin) = expr {
        matches!(bin.operator, BinaryOperator::And | BinaryOperator::Or)
    } else {
        false
    }
}

/// Check if an expression is a nullish coalescing binary operation.
///
/// Used to detect cases where parentheses are required when mixing with `&&`/`||`.
fn is_nullish_coalesce(expr: &OutputExpression<'_>) -> bool {
    if let OutputExpression::BinaryOperator(bin) = expr {
        matches!(bin.operator, BinaryOperator::NullishCoalesce)
    } else {
        false
    }
}

/// Escape a string for JavaScript output.
///
/// Uses double quotes to match Angular's output style.
/// Escapes `"`, `\`, `\n`, `\r`, `$` (when requested), ASCII control characters,
/// and all non-ASCII characters (code point > 0x7E) as `\uNNNN` sequences.
/// Characters above the BMP (U+10000+) are encoded as UTF-16 surrogate pairs
/// (`\uXXXX\uXXXX`). This matches TypeScript's emitter behavior, which escapes
/// non-ASCII characters in string literals.
fn escape_string(input: &str, escape_dollar: bool) -> String {
    let mut result = String::with_capacity(input.len() + 2);
    result.push('"');
    for c in input.chars() {
        match c {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '$' if escape_dollar => result.push_str("\\$"),
            // ASCII printable characters (0x20-0x7E) are emitted literally
            c if (' '..='\x7E').contains(&c) => result.push(c),
            // Everything else (ASCII control chars, non-ASCII) is escaped as \uNNNN.
            // Characters above the BMP are encoded as UTF-16 surrogate pairs.
            c => {
                let code = c as u32;
                if code <= 0xFFFF {
                    push_unicode_escape(&mut result, code);
                } else {
                    let hi = 0xD800 + ((code - 0x10000) >> 10);
                    let lo = 0xDC00 + ((code - 0x10000) & 0x3FF);
                    push_unicode_escape(&mut result, hi);
                    push_unicode_escape(&mut result, lo);
                }
            }
        }
    }
    result.push('"');
    result
}

/// Push a `\uXXXX` escape sequence for a 16-bit code unit.
fn push_unicode_escape(buf: &mut String, code: u32) {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    buf.push_str("\\u");
    buf.push(HEX[((code >> 12) & 0xF) as usize] as char);
    buf.push(HEX[((code >> 8) & 0xF) as usize] as char);
    buf.push(HEX[((code >> 4) & 0xF) as usize] as char);
    buf.push(HEX[(code & 0xF) as usize] as char);
}

/// Escape an identifier for use as a property key.
fn escape_identifier(input: &Atom<'_>, escape_dollar: bool, always_quote: bool) -> String {
    // Check if the identifier is a valid JavaScript identifier
    fn is_legal_identifier(s: &str) -> bool {
        let mut chars = s.chars();
        // Use if-let pattern to avoid unwrap - first char must exist and be valid
        let Some(first) = chars.next() else {
            return false;
        };
        if !first.is_alphabetic() && first != '_' && first != '$' {
            return false;
        }
        chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
    }

    if always_quote || !is_legal_identifier(input) {
        escape_string(input, escape_dollar)
    } else {
        input.to_string()
    }
}

/// Format a number exactly like JavaScript's `Number.prototype.toString()`.
///
/// JavaScript and Rust differ in their formatting of `f64` values:
/// - JS uses scientific notation for exponents >= 21 (e.g., `1e+21`), Rust uses decimal
/// - JS uses scientific notation for very small numbers (e.g., `1e-7`), Rust uses decimal
/// - JS outputs `Infinity`/`-Infinity`, Rust outputs `inf`/`-inf`
/// - JS outputs `0` for negative zero, Rust outputs `-0`
///
/// This function matches the ECMAScript specification for `Number::toString()`
/// (ECMA-262, 7.1.12.1) to ensure the emitted code matches Angular's TypeScript compiler
/// which uses JavaScript's template literal coercion (`${value}`).
fn format_number_like_js(value: f64) -> String {
    // 1. NaN
    if value.is_nan() {
        return "NaN".to_string();
    }
    // 2. +0 or -0 => "0"
    if value == 0.0 {
        return "0".to_string();
    }
    // 3. Negative: prepend "-" and format the absolute value
    if value < 0.0 {
        return format!("-{}", format_number_like_js(-value));
    }
    // 4. Infinity
    if value.is_infinite() {
        return "Infinity".to_string();
    }

    // 5. For finite positive numbers, extract the significant digits and exponent.
    //
    // We use Rust's {:e} (scientific notation) formatting to get the shortest
    // representation in a form we can parse: "d.dddde±N" or "deN".
    let sci = format!("{value:e}");

    // Parse the scientific notation string to extract digits and exponent.
    // Format is like "3.14159265358979e0" or "1e-7" or "1.5e21"
    let (mantissa_str, exp_str) = sci.split_once('e').unwrap_or((&sci, "0"));
    let exp: i32 = exp_str.parse().unwrap_or(0);

    // Extract all significant digits (removing the decimal point)
    let digits: String = mantissa_str.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32; // number of significant digits

    // n is the position of the decimal point relative to the first digit.
    // In scientific notation a.bcd * 10^e, the integer is abcd (k=4 digits)
    // and value = abcd * 10^(e - k + 1), so n = e + 1 (where n means:
    // digits represent an integer s, and value = s * 10^(n-k))
    let n = exp + 1;

    // 6. Format according to ECMAScript spec rules:
    if k <= n && n <= 21 {
        // Case: k <= n <= 21
        // Example: 1e8 -> digits="1", k=1, n=9 -> "1" + "00000000" = "100000000"
        let mut result = digits;
        for _ in 0..(n - k) {
            result.push('0');
        }
        result
    } else if 0 < n && n <= 21 {
        // Case: 0 < n <= 21 (and n < k since we passed the first case)
        // Example: 42.5 -> digits="425", k=3, n=2 -> "42.5"
        let n = n as usize;
        format!("{}.{}", &digits[..n], &digits[n..])
    } else if -6 < n && n <= 0 {
        // Case: -6 < n <= 0
        // Example: 0.000001 (1e-6) -> digits="1", k=1, exp=-6, n=-5
        // Format: "0." + "0"*(-n) + digits
        let zeros = "0".repeat((-n) as usize);
        format!("0.{zeros}{digits}")
    } else if k == 1 {
        // Single digit with scientific notation
        // Example: 1e+21 -> digits="1", k=1, n=22
        let exp_val = n - 1;
        if exp_val > 0 { format!("{digits}e+{exp_val}") } else { format!("{digits}e{exp_val}") }
    } else {
        // Multiple digits with scientific notation
        // Example: 1.5e+21 -> digits="15", k=2, n=22
        let exp_val = n - 1;
        if exp_val > 0 {
            format!("{}e+{exp_val}", format!("{}.{}", &digits[..1], &digits[1..]))
        } else {
            format!("{}e{exp_val}", format!("{}.{}", &digits[..1], &digits[1..]))
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::output::ast::{LiteralExpr, LiteralValue, ReadVarExpr};
    use oxc_allocator::{Allocator, Box};
    use oxc_span::Atom;

    #[test]
    fn test_emit_literal_null() {
        let emitter = JsEmitter::new();
        let alloc = Allocator::default();
        let expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Null, source_span: None },
            &alloc,
        ));
        assert_eq!(emitter.emit_expression(&expr), "null");
    }

    #[test]
    fn test_emit_literal_boolean() {
        let emitter = JsEmitter::new();
        let alloc = Allocator::default();
        let expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(true), source_span: None },
            &alloc,
        ));
        assert_eq!(emitter.emit_expression(&expr), "true");
    }

    #[test]
    fn test_emit_literal_number() {
        let emitter = JsEmitter::new();
        let alloc = Allocator::default();
        let expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(42.5), source_span: None },
            &alloc,
        ));
        assert_eq!(emitter.emit_expression(&expr), "42.5");
    }

    #[test]
    fn test_format_number_like_js() {
        // Leading zero for decimals: "0.3" not ".3"
        assert_eq!(format_number_like_js(0.3), "0.3");

        // Integer: no decimal point
        assert_eq!(format_number_like_js(42.0), "42");

        // Decimal
        assert_eq!(format_number_like_js(42.5), "42.5");

        // Negative integer
        assert_eq!(format_number_like_js(-1.0), "-1");

        // Zero (including negative zero)
        assert_eq!(format_number_like_js(0.0), "0");
        assert_eq!(format_number_like_js(-0.0), "0");

        // Large integer: decimal notation, not scientific
        assert_eq!(format_number_like_js(1e8), "100000000");
        assert_eq!(format_number_like_js(1e20), "100000000000000000000");

        // Very large: scientific notation with e+
        assert_eq!(format_number_like_js(1e21), "1e+21");
        assert_eq!(format_number_like_js(1.5e21), "1.5e+21");

        // Small numbers: JS uses scientific notation for exponent < -6
        assert_eq!(format_number_like_js(1e-7), "1e-7");
        assert_eq!(format_number_like_js(5e-7), "5e-7");
        assert_eq!(format_number_like_js(1.5e-7), "1.5e-7");

        // Small numbers: decimal notation for exponent >= -6
        assert_eq!(format_number_like_js(1e-6), "0.000001");
        assert_eq!(format_number_like_js(0.1), "0.1");
        assert_eq!(format_number_like_js(0.01), "0.01");

        // Special values
        assert_eq!(format_number_like_js(f64::NAN), "NaN");
        assert_eq!(format_number_like_js(f64::INFINITY), "Infinity");
        assert_eq!(format_number_like_js(f64::NEG_INFINITY), "-Infinity");

        // Negative decimal
        assert_eq!(format_number_like_js(-0.3), "-0.3");
    }

    #[test]
    fn test_emit_literal_string() {
        let emitter = JsEmitter::new();
        let alloc = Allocator::default();
        let expr = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::String(Atom::from("hello")), source_span: None },
            &alloc,
        ));
        // Uses double quotes to match Angular's output style
        assert_eq!(emitter.emit_expression(&expr), "\"hello\"");
    }

    #[test]
    fn test_emit_variable() {
        let emitter = JsEmitter::new();
        let alloc = Allocator::default();
        let expr = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("myVar"), source_span: None },
            &alloc,
        ));
        assert_eq!(emitter.emit_expression(&expr), "myVar");
    }

    #[test]
    fn test_escape_string() {
        // Uses double quotes to match Angular's output style
        assert_eq!(escape_string("hello", false), "\"hello\"");
        assert_eq!(escape_string("it's", false), "\"it's\"");
        assert_eq!(escape_string("say \"hi\"", false), "\"say \\\"hi\\\"\"");
        assert_eq!(escape_string("line\nbreak", false), "\"line\\nbreak\"");
        assert_eq!(escape_string("$var", true), "\"\\$var\"");
        assert_eq!(escape_string("$var", false), "\"$var\"");
    }

    #[test]
    fn test_escape_string_unicode_literals() {
        // Non-ASCII characters should be escaped as \uNNNN to match
        // TypeScript's emitter behavior.

        // &times; (multiplication sign U+00D7) -> \u00D7
        assert_eq!(escape_string("\u{00D7}", false), "\"\\u00D7\"");

        // &nbsp; (non-breaking space U+00A0) -> \u00A0
        assert_eq!(escape_string("\u{00A0}", false), "\"\\u00A0\"");

        // Mixed ASCII and non-ASCII
        assert_eq!(escape_string("a\u{00D7}b", false), "\"a\\u00D7b\"");

        // Multiple non-ASCII characters
        assert_eq!(escape_string("\u{00D7}\u{00A0}", false), "\"\\u00D7\\u00A0\"");

        // Characters outside BMP (emoji) -> surrogate pair
        assert_eq!(escape_string("\u{1F600}", false), "\"\\uD83D\\uDE00\"");

        // Common HTML entities -> all escaped as \uNNNN
        assert_eq!(escape_string("\u{00A9}", false), "\"\\u00A9\""); // &copy; ©
        assert_eq!(escape_string("\u{00AE}", false), "\"\\u00AE\""); // &reg; ®
        assert_eq!(escape_string("\u{2014}", false), "\"\\u2014\""); // &mdash; —
        assert_eq!(escape_string("\u{2013}", false), "\"\\u2013\""); // &ndash; –

        // Greek letter alpha
        assert_eq!(escape_string("\u{03B1}", false), "\"\\u03B1\""); // α

        // Accented Latin letter
        assert_eq!(escape_string("\u{00E9}", false), "\"\\u00E9\""); // é
    }

    #[test]
    fn test_escape_string_control_characters() {
        // ASCII control characters (other than \n and \r) should be escaped
        assert_eq!(escape_string("\u{0000}", false), "\"\\u0000\""); // NUL
        assert_eq!(escape_string("\u{0001}", false), "\"\\u0001\""); // SOH
        assert_eq!(escape_string("\u{0008}", false), "\"\\u0008\""); // BS
        assert_eq!(escape_string("\u{000B}", false), "\"\\u000B\""); // VT
        assert_eq!(escape_string("\u{001F}", false), "\"\\u001F\""); // US
        assert_eq!(escape_string("\u{007F}", false), "\"\\u007F\""); // DEL

        // \n and \r have their own named escapes
        assert_eq!(escape_string("\n", false), "\"\\n\"");
        assert_eq!(escape_string("\r", false), "\"\\r\"");
    }

    #[test]
    fn test_escape_string_non_ascii_as_unicode_escapes() {
        // Non-ASCII characters should be escaped as \uNNNN to match
        // TypeScript's emitter behavior (which escapes non-ASCII in string literals).

        // Non-breaking space U+00A0
        assert_eq!(escape_string("\u{00A0}", false), "\"\\u00A0\"");

        // En dash U+2013
        assert_eq!(escape_string("\u{2013}", false), "\"\\u2013\"");

        // Trademark U+2122
        assert_eq!(escape_string("\u{2122}", false), "\"\\u2122\"");

        // Infinity U+221E
        assert_eq!(escape_string("\u{221E}", false), "\"\\u221E\"");

        // Mixed ASCII and non-ASCII
        assert_eq!(escape_string("a\u{00D7}b", false), "\"a\\u00D7b\"");

        // Multiple non-ASCII characters
        assert_eq!(escape_string("\u{00D7}\u{00A0}", false), "\"\\u00D7\\u00A0\"");

        // Characters above BMP should use surrogate pairs
        // U+1F600 (grinning face) = surrogate pair D83D DE00
        assert_eq!(escape_string("\u{1F600}", false), "\"\\uD83D\\uDE00\"");

        // U+10000 (first supplementary char) = surrogate pair D800 DC00
        assert_eq!(escape_string("\u{10000}", false), "\"\\uD800\\uDC00\"");

        // ASCII printable chars (0x20-0x7E) should remain literal
        assert_eq!(escape_string(" ~", false), "\" ~\"");
        assert_eq!(escape_string("abc123!@#", false), "\"abc123!@#\"");
    }

    // ========================================================================
    // Source Map Tests
    // ========================================================================

    use crate::util::{ParseLocation, ParseSourceFile};
    use std::sync::Arc;

    fn make_source_file(content: &str) -> Arc<ParseSourceFile> {
        Arc::new(ParseSourceFile::new(content, "test.ts"))
    }

    fn make_span(file: &Arc<ParseSourceFile>, line: u32, col: u32, offset: u32) -> ParseSourceSpan {
        let loc = ParseLocation::new(file.clone(), offset, line, col);
        ParseSourceSpan::new(loc.clone(), loc)
    }

    #[test]
    fn test_source_map_no_source_returns_none() {
        let ctx = EmitterContext::new();
        assert!(ctx.to_source_map(None).is_none());
    }

    #[test]
    fn test_source_map_with_source_but_no_mappings() {
        let file = make_source_file("let x = 1;");
        let ctx = EmitterContext::with_source_file(file);
        assert!(ctx.to_source_map(None).is_none());
    }

    #[test]
    fn test_source_map_basic_mapping() {
        let file = make_source_file("let x = 1;");
        let mut ctx = EmitterContext::with_source_file(file.clone());

        let span = make_span(&file, 0, 0, 0);
        ctx.print_with_span("let x = 1;", Some(span));

        let map = ctx.to_source_map(Some("test.js")).expect("should generate source map");

        // Verify the source map has the expected structure
        assert!(map.get_sources().any(|s| s.as_ref() == "test.ts"));
        assert!(map.get_tokens().count() > 0);
    }

    #[test]
    fn test_source_map_multiple_lines() {
        let file = make_source_file("let x = 1;\nlet y = 2;");
        let mut ctx = EmitterContext::with_source_file(file.clone());

        let span1 = make_span(&file, 0, 0, 0);
        let span2 = make_span(&file, 1, 0, 11);

        ctx.print_with_span("let x = 1;", Some(span1));
        ctx.println("");
        ctx.print_with_span("let y = 2;", Some(span2));

        let map = ctx.to_source_map(Some("test.js")).expect("should generate source map");

        // Should have 2 tokens (one for each statement)
        assert_eq!(map.get_tokens().count(), 2);
    }

    #[test]
    fn test_source_map_deduplicates_consecutive_same_spans() {
        let file = make_source_file("let x = 1;");
        let mut ctx = EmitterContext::with_source_file(file.clone());

        // Same span printed multiple times in a row
        let span = make_span(&file, 0, 0, 0);
        ctx.print_with_span("let", Some(span.clone()));
        ctx.print_with_span(" ", Some(span.clone()));
        ctx.print_with_span("x", Some(span.clone()));
        ctx.print_with_span(" = ", Some(span.clone()));
        ctx.print_with_span("1", Some(span));

        let map = ctx.to_source_map(Some("test.js")).expect("should generate source map");

        // Should only have 1 token since all spans are the same
        assert_eq!(map.get_tokens().count(), 1);
    }

    #[test]
    fn test_source_map_different_spans_creates_multiple_tokens() {
        let file = make_source_file("let x = y + z;");
        let mut ctx = EmitterContext::with_source_file(file.clone());

        let span1 = make_span(&file, 0, 8, 8); // 'y'
        let span2 = make_span(&file, 0, 12, 12); // 'z'

        ctx.print("let x = ");
        ctx.print_with_span("y", Some(span1));
        ctx.print(" + ");
        ctx.print_with_span("z", Some(span2));

        let map = ctx.to_source_map(Some("test.js")).expect("should generate source map");

        // Should have 2 tokens for y and z
        assert_eq!(map.get_tokens().count(), 2);
    }

    #[test]
    fn test_source_and_map_together() {
        let file = make_source_file("let x = 1;");
        let mut ctx = EmitterContext::with_source_file(file.clone());

        let span = make_span(&file, 0, 0, 0);
        ctx.print_with_span("let x = 1;", Some(span));

        let (source, map) = ctx.to_source_with_map(Some("test.js"));

        assert_eq!(source, "let x = 1;");
        assert!(map.is_some());
    }

    #[test]
    fn test_source_map_with_indentation() {
        let file = make_source_file("function foo() {\n  return 1;\n}");
        let mut ctx = EmitterContext::with_source_file(file.clone());

        ctx.print("function foo() {");
        ctx.println("");
        ctx.inc_indent();
        let span = make_span(&file, 1, 2, 19); // 'return 1;'
        ctx.print_with_span("return 1;", Some(span));
        ctx.println("");
        ctx.dec_indent();
        ctx.print("}");

        let (source, map) = ctx.to_source_with_map(Some("test.js"));

        assert!(source.contains("  return 1;")); // indented with 2 spaces
        assert!(map.is_some());
    }

    #[test]
    fn test_span_to_source_span_conversion() {
        let file = make_source_file("let x = 1;\nlet y = 2;");
        let ctx = EmitterContext::with_source_file(file);

        // Test converting byte offsets to line/column
        let span = Span::new(0, 10); // "let x = 1;"
        let source_span = ctx.span_to_source_span(span).expect("should convert span");

        assert_eq!(source_span.start.line, 0);
        assert_eq!(source_span.start.col, 0);
        assert_eq!(source_span.start.offset, 0);
        assert_eq!(source_span.end.offset, 10);
    }

    #[test]
    fn test_span_to_source_span_second_line() {
        let file = make_source_file("let x = 1;\nlet y = 2;");
        let ctx = EmitterContext::with_source_file(file);

        // Test span on second line (after newline at offset 10)
        let span = Span::new(11, 21); // "let y = 2;"
        let source_span = ctx.span_to_source_span(span).expect("should convert span");

        assert_eq!(source_span.start.line, 1);
        assert_eq!(source_span.start.col, 0);
        assert_eq!(source_span.start.offset, 11);
    }

    #[test]
    fn test_span_to_source_span_no_source_file() {
        let ctx = EmitterContext::new();

        // Without a source file, span conversion should return None
        let span = Span::new(0, 10);
        assert!(ctx.span_to_source_span(span).is_none());
    }

    #[test]
    fn test_emit_jsdoc_comment() {
        use super::super::ast::{DeclareVarStmt, JsDocComment, LeadingComment, StmtModifier};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();

        // Create a statement with a JSDoc comment
        let stmt = DeclareVarStmt {
            name: Atom::from("MSG_HELLO"),
            value: None,
            modifiers: StmtModifier::FINAL,
            leading_comment: Some(LeadingComment::JSDoc(JsDocComment {
                description: Some(Atom::from("Hello world")),
                meaning: Some(Atom::from("greeting")),
                suppress_msg_descriptions: false,
            })),
            source_span: None,
        };

        let output = emitter.emit_statement(&crate::output::ast::OutputStatement::DeclareVar(
            oxc_allocator::Box::new_in(stmt, &oxc_allocator::Allocator::default()),
        ));

        assert!(output.contains("/** @desc Hello world @meaning greeting */"));
        assert!(output.contains("const MSG_HELLO;"));
    }

    #[test]
    fn test_emit_jsdoc_with_suppress() {
        use super::super::ast::{DeclareVarStmt, JsDocComment, LeadingComment, StmtModifier};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();

        // Create a statement with @suppress
        let stmt = DeclareVarStmt {
            name: Atom::from("MSG_HELLO"),
            value: None,
            modifiers: StmtModifier::FINAL,
            leading_comment: Some(LeadingComment::JSDoc(JsDocComment {
                description: None,
                meaning: None,
                suppress_msg_descriptions: true,
            })),
            source_span: None,
        };

        let output = emitter.emit_statement(&crate::output::ast::OutputStatement::DeclareVar(
            oxc_allocator::Box::new_in(stmt, &oxc_allocator::Allocator::default()),
        ));

        assert!(output.contains("/** @suppress {msgDescriptions} */"));
        assert!(output.contains("const MSG_HELLO;"));
    }

    #[test]
    fn test_emit_single_line_comment() {
        use super::super::ast::{DeclareVarStmt, LeadingComment, StmtModifier};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();

        let stmt = DeclareVarStmt {
            name: Atom::from("x"),
            value: None,
            modifiers: StmtModifier::NONE,
            leading_comment: Some(LeadingComment::SingleLine(Atom::from("test comment"))),
            source_span: None,
        };

        let output = emitter.emit_statement(&crate::output::ast::OutputStatement::DeclareVar(
            oxc_allocator::Box::new_in(stmt, &oxc_allocator::Allocator::default()),
        ));

        assert!(output.contains("// test comment"));
        assert!(output.contains("let x;"));
    }

    #[test]
    fn test_emit_multi_line_comment() {
        use super::super::ast::{DeclareVarStmt, LeadingComment, StmtModifier};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();

        let stmt = DeclareVarStmt {
            name: Atom::from("x"),
            value: None,
            modifiers: StmtModifier::NONE,
            leading_comment: Some(LeadingComment::MultiLine(Atom::from("multi\nline"))),
            source_span: None,
        };

        let output = emitter.emit_statement(&crate::output::ast::OutputStatement::DeclareVar(
            oxc_allocator::Box::new_in(stmt, &oxc_allocator::Allocator::default()),
        ));

        // Multi-line comments get " * " prefix on continuation lines
        assert!(output.contains("/*multi\n * line */"));
        assert!(output.contains("let x;"));
    }

    #[test]
    fn test_emit_multi_line_comment_with_asterisk() {
        use super::super::ast::{DeclareVarStmt, LeadingComment, StmtModifier};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();

        // Test license-style comment with asterisks
        let stmt = DeclareVarStmt {
            name: Atom::from("x"),
            value: None,
            modifiers: StmtModifier::NONE,
            leading_comment: Some(LeadingComment::MultiLine(Atom::from(
                "\n* @license\n* Copyright Google LLC\n",
            ))),
            source_span: None,
        };

        let output = emitter.emit_statement(&crate::output::ast::OutputStatement::DeclareVar(
            oxc_allocator::Box::new_in(stmt, &oxc_allocator::Allocator::default()),
        ));

        // Should have space before * on each continuation line
        assert!(output.contains(" * @license"));
        assert!(output.contains(" * Copyright Google LLC"));
        assert!(output.contains("let x;"));
    }

    #[test]
    fn test_emit_conditional_expression() {
        use super::super::ast::{ConditionalExpr, LiteralExpr, LiteralValue, ReadVarExpr};
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: (condition? true: false)
        let condition = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("condition"), source_span: None },
            &alloc,
        ));
        let true_case = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(2.0), source_span: None },
            &alloc,
        ));
        let false_case = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(-1.0), source_span: None },
            &alloc,
        ));

        let expr = OutputExpression::Conditional(Box::new_in(
            ConditionalExpr {
                condition: Box::new_in(condition, &alloc),
                true_case: Box::new_in(true_case, &alloc),
                false_case: Some(Box::new_in(false_case, &alloc)),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Matches Angular compiler output format: no space before ?, space after ? and :
        assert_eq!(output, "(condition? 2: -1)");
    }

    #[test]
    fn test_emit_nested_conditional_expression() {
        use super::super::ast::{
            BinaryOperatorExpr, ConditionalExpr, LiteralExpr, LiteralValue, ReadVarExpr,
        };
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: ((tmp === "month")? 2: ((tmp === "year")? 3: -1))
        // Inner conditional: (tmp === "year")? 3: -1
        let inner_condition = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::Identical,
                lhs: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Atom::from("tmp"), source_span: None },
                        &alloc,
                    )),
                    &alloc,
                ),
                rhs: Box::new_in(
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::String(Atom::from("year")),
                            source_span: None,
                        },
                        &alloc,
                    )),
                    &alloc,
                ),
                source_span: None,
            },
            &alloc,
        ));

        let inner_cond_expr = OutputExpression::Conditional(Box::new_in(
            ConditionalExpr {
                condition: Box::new_in(inner_condition, &alloc),
                true_case: Box::new_in(
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Number(3.0), source_span: None },
                        &alloc,
                    )),
                    &alloc,
                ),
                false_case: Some(Box::new_in(
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Number(-1.0), source_span: None },
                        &alloc,
                    )),
                    &alloc,
                )),
                source_span: None,
            },
            &alloc,
        ));

        // Outer conditional: (tmp === "month")? 2: inner
        let outer_condition = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::Identical,
                lhs: Box::new_in(
                    OutputExpression::ReadVar(Box::new_in(
                        ReadVarExpr { name: Atom::from("tmp"), source_span: None },
                        &alloc,
                    )),
                    &alloc,
                ),
                rhs: Box::new_in(
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr {
                            value: LiteralValue::String(Atom::from("month")),
                            source_span: None,
                        },
                        &alloc,
                    )),
                    &alloc,
                ),
                source_span: None,
            },
            &alloc,
        ));

        let expr = OutputExpression::Conditional(Box::new_in(
            ConditionalExpr {
                condition: Box::new_in(outer_condition, &alloc),
                true_case: Box::new_in(
                    OutputExpression::Literal(Box::new_in(
                        LiteralExpr { value: LiteralValue::Number(2.0), source_span: None },
                        &alloc,
                    )),
                    &alloc,
                ),
                false_case: Some(Box::new_in(inner_cond_expr, &alloc)),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Matches Angular compiler output format for nested conditionals (uses double quotes)
        assert_eq!(output, "((tmp === \"month\")? 2: ((tmp === \"year\")? 3: -1))");
    }

    #[test]
    fn test_emit_spread_element_in_array() {
        use super::super::ast::{LiteralArrayExpr, SpreadElementExpr};

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: [...arr, 1, 2]
        let arr_var = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("arr"), source_span: None },
            &alloc,
        ));
        let spread_expr = OutputExpression::SpreadElement(Box::new_in(
            SpreadElementExpr { expr: Box::new_in(arr_var, &alloc), source_span: None },
            &alloc,
        ));
        let one = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(1.0), source_span: None },
            &alloc,
        ));
        let two = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(2.0), source_span: None },
            &alloc,
        ));

        let mut entries = oxc_allocator::Vec::new_in(&alloc);
        entries.push(spread_expr);
        entries.push(one);
        entries.push(two);

        let array_expr = OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries, source_span: None },
            &alloc,
        ));

        let output = emitter.emit_expression(&array_expr);
        assert_eq!(output, "[...arr,1,2]");
    }

    #[test]
    fn test_emit_multiple_spread_elements() {
        use super::super::ast::{LiteralArrayExpr, SpreadElementExpr};

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: [...a, ...b]
        let a_var = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("a"), source_span: None },
            &alloc,
        ));
        let b_var = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("b"), source_span: None },
            &alloc,
        ));
        let spread_a = OutputExpression::SpreadElement(Box::new_in(
            SpreadElementExpr { expr: Box::new_in(a_var, &alloc), source_span: None },
            &alloc,
        ));
        let spread_b = OutputExpression::SpreadElement(Box::new_in(
            SpreadElementExpr { expr: Box::new_in(b_var, &alloc), source_span: None },
            &alloc,
        ));

        let mut entries = oxc_allocator::Vec::new_in(&alloc);
        entries.push(spread_a);
        entries.push(spread_b);

        let array_expr = OutputExpression::LiteralArray(Box::new_in(
            LiteralArrayExpr { entries, source_span: None },
            &alloc,
        ));

        let output = emitter.emit_expression(&array_expr);
        assert_eq!(output, "[...a,...b]");
    }

    // ========================================================================
    // Nullish Coalescing / Logical Operator Mixing Tests
    // ========================================================================

    #[test]
    fn test_emit_nullish_coalescing_with_logical_and_on_left() {
        use super::super::ast::BinaryOperatorExpr;
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: (a && b) ?? c
        let a = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("a"), source_span: None },
            &alloc,
        ));
        let b = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("b"), source_span: None },
            &alloc,
        ));
        let c = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("c"), source_span: None },
            &alloc,
        ));

        // a && b
        let and_expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::And,
                lhs: Box::new_in(a, &alloc),
                rhs: Box::new_in(b, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        // (a && b) ?? c
        let expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::NullishCoalesce,
                lhs: Box::new_in(and_expr, &alloc),
                rhs: Box::new_in(c, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // The logical && on left of ?? needs extra parentheses
        assert_eq!(output, "(((a && b)) ?? c)");
    }

    #[test]
    fn test_emit_nullish_coalescing_with_logical_or_on_right() {
        use super::super::ast::BinaryOperatorExpr;
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: a ?? (b || c)
        let a = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("a"), source_span: None },
            &alloc,
        ));
        let b = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("b"), source_span: None },
            &alloc,
        ));
        let c = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("c"), source_span: None },
            &alloc,
        ));

        // b || c
        let or_expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::Or,
                lhs: Box::new_in(b, &alloc),
                rhs: Box::new_in(c, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        // a ?? (b || c)
        let expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::NullishCoalesce,
                lhs: Box::new_in(a, &alloc),
                rhs: Box::new_in(or_expr, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // The logical || on right of ?? needs extra parentheses
        assert_eq!(output, "(a ?? ((b || c)))");
    }

    #[test]
    fn test_emit_logical_and_with_nullish_coalescing_on_left() {
        use super::super::ast::BinaryOperatorExpr;
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: (a ?? b) && c
        let a = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("a"), source_span: None },
            &alloc,
        ));
        let b = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("b"), source_span: None },
            &alloc,
        ));
        let c = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("c"), source_span: None },
            &alloc,
        ));

        // a ?? b
        let nullish_expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::NullishCoalesce,
                lhs: Box::new_in(a, &alloc),
                rhs: Box::new_in(b, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        // (a ?? b) && c
        let expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::And,
                lhs: Box::new_in(nullish_expr, &alloc),
                rhs: Box::new_in(c, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // The ?? on left of && needs extra parentheses
        assert_eq!(output, "(((a ?? b)) && c)");
    }

    #[test]
    fn test_emit_nullish_coalescing_with_conditional_on_left() {
        use super::super::ast::{BinaryOperatorExpr, ConditionalExpr};
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: (a ? b : c) ?? d
        let a = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("a"), source_span: None },
            &alloc,
        ));
        let b = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("b"), source_span: None },
            &alloc,
        ));
        let c = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("c"), source_span: None },
            &alloc,
        ));
        let d = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("d"), source_span: None },
            &alloc,
        ));

        // a ? b : c
        let cond_expr = OutputExpression::Conditional(Box::new_in(
            ConditionalExpr {
                condition: Box::new_in(a, &alloc),
                true_case: Box::new_in(b, &alloc),
                false_case: Some(Box::new_in(c, &alloc)),
                source_span: None,
            },
            &alloc,
        ));

        // (a ? b : c) ?? d
        let expr = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::NullishCoalesce,
                lhs: Box::new_in(cond_expr, &alloc),
                rhs: Box::new_in(d, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // The conditional on left of ?? needs extra parentheses
        assert_eq!(output, "(((a? b: c)) ?? d)");
    }

    // ========================================================================
    // Arrow Function Paren Tests
    // ========================================================================

    #[test]
    fn test_emit_arrow_function_single_param_with_parens() {
        use super::super::ast::{
            ArrowFunctionBody, ArrowFunctionExpr, BinaryOperatorExpr, FnParam,
        };

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: (x) =>(x + 1)
        let x_var = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("x"), source_span: None },
            &alloc,
        ));
        let one = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(1.0), source_span: None },
            &alloc,
        ));
        let body = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::Plus,
                lhs: Box::new_in(x_var, &alloc),
                rhs: Box::new_in(one, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        let mut params = oxc_allocator::Vec::new_in(&alloc);
        params.push(FnParam { name: Atom::from("x") });

        let expr = OutputExpression::ArrowFunction(Box::new_in(
            ArrowFunctionExpr {
                params,
                body: ArrowFunctionBody::Expression(Box::new_in(body, &alloc)),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Single param: always wrap in parens, matches Angular's abstract_js_emitter behavior
        assert_eq!(output, "(x) =>(x + 1)");
    }

    #[test]
    fn test_emit_arrow_function_multiple_params_with_parens() {
        use super::super::ast::{
            ArrowFunctionBody, ArrowFunctionExpr, BinaryOperatorExpr, FnParam,
        };

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: (x, y) =>(x + y)
        let x_var = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("x"), source_span: None },
            &alloc,
        ));
        let y_var = OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("y"), source_span: None },
            &alloc,
        ));
        let body = OutputExpression::BinaryOperator(Box::new_in(
            BinaryOperatorExpr {
                operator: super::super::ast::BinaryOperator::Plus,
                lhs: Box::new_in(x_var, &alloc),
                rhs: Box::new_in(y_var, &alloc),
                source_span: None,
            },
            &alloc,
        ));

        let mut params = oxc_allocator::Vec::new_in(&alloc);
        params.push(FnParam { name: Atom::from("x") });
        params.push(FnParam { name: Atom::from("y") });

        let expr = OutputExpression::ArrowFunction(Box::new_in(
            ArrowFunctionExpr {
                params,
                body: ArrowFunctionBody::Expression(Box::new_in(body, &alloc)),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Multiple params: with parens
        assert_eq!(output, "(x,y) =>(x + y)");
    }

    #[test]
    fn test_emit_arrow_function_zero_params_with_parens() {
        use super::super::ast::{ArrowFunctionBody, ArrowFunctionExpr};

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Build: () =>42
        let body = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Number(42.0), source_span: None },
            &alloc,
        ));

        let params = oxc_allocator::Vec::new_in(&alloc);

        let expr = OutputExpression::ArrowFunction(Box::new_in(
            ArrowFunctionExpr {
                params,
                body: ArrowFunctionBody::Expression(Box::new_in(body, &alloc)),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Zero params: with parens
        assert_eq!(output, "() =>42");
    }

    #[test]
    fn test_emit_localized_string_has_space_before_paren() {
        use crate::output::ast::LocalizedStringExpr;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // Simple $localize with a single message part and no expressions
        let mut message_parts = oxc_allocator::Vec::new_in(&alloc);
        message_parts.push(Atom::from("Hello"));

        let placeholder_names = oxc_allocator::Vec::new_in(&alloc);
        let expressions = oxc_allocator::Vec::new_in(&alloc);

        let expr = OutputExpression::LocalizedString(Box::new_in(
            LocalizedStringExpr {
                description: None,
                meaning: None,
                custom_id: None,
                message_parts,
                placeholder_names,
                expressions,
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Must emit "$localize(" without a space before the opening paren
        assert!(output.starts_with("$localize("), "Expected '$localize(' but got: {output}");
    }

    #[test]
    fn test_emit_localized_string_with_expressions() {
        use crate::output::ast::LocalizedStringExpr;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        // $localize with interpolation: "Hello {$name}!"
        let mut message_parts = oxc_allocator::Vec::new_in(&alloc);
        message_parts.push(Atom::from("Hello "));
        message_parts.push(Atom::from("!"));

        let mut placeholder_names = oxc_allocator::Vec::new_in(&alloc);
        placeholder_names.push(Atom::from("name"));

        let mut expressions = oxc_allocator::Vec::new_in(&alloc);
        expressions.push(OutputExpression::ReadVar(Box::new_in(
            ReadVarExpr { name: Atom::from("name"), source_span: None },
            &alloc,
        )));

        let expr = OutputExpression::LocalizedString(Box::new_in(
            LocalizedStringExpr {
                description: None,
                meaning: None,
                custom_id: None,
                message_parts,
                placeholder_names,
                expressions,
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        // Must not have space before paren and include the expression
        assert!(output.starts_with("$localize("), "Expected '$localize(' but got: {output}");
        assert!(output.contains(", name)"), "Expected expression argument but got: {output}");
    }

    // ========================================================================
    // Empty Body Tests
    // ========================================================================

    #[test]
    fn test_emit_empty_function_expression_body() {
        use super::super::ast::FunctionExpr;
        use oxc_allocator::{Allocator, Box};

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        let expr = OutputExpression::Function(Box::new_in(
            FunctionExpr {
                name: None,
                params: oxc_allocator::Vec::new_in(&alloc),
                statements: oxc_allocator::Vec::new_in(&alloc),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        assert_eq!(output, "function() {\n}");
    }

    #[test]
    fn test_emit_empty_arrow_function_statement_body() {
        use super::super::ast::ArrowFunctionExpr;
        use oxc_allocator::{Allocator, Box};

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        let expr = OutputExpression::ArrowFunction(Box::new_in(
            ArrowFunctionExpr {
                params: oxc_allocator::Vec::new_in(&alloc),
                body: ArrowFunctionBody::Statements(oxc_allocator::Vec::new_in(&alloc)),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_expression(&expr);
        assert_eq!(output, "() =>{\n}");
    }

    #[test]
    fn test_emit_empty_if_body() {
        use super::super::ast::{IfStmt, LiteralExpr, LiteralValue};
        use oxc_allocator::{Allocator, Box};

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        let condition = OutputExpression::Literal(Box::new_in(
            LiteralExpr { value: LiteralValue::Boolean(true), source_span: None },
            &alloc,
        ));

        let stmt = OutputStatement::If(Box::new_in(
            IfStmt {
                condition,
                true_case: oxc_allocator::Vec::new_in(&alloc),
                false_case: oxc_allocator::Vec::new_in(&alloc),
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_statement(&stmt);
        assert_eq!(output, "if (true) {  }");
    }

    #[test]
    fn test_emit_empty_declare_function_body() {
        use super::super::ast::{DeclareFunctionStmt, StmtModifier};
        use oxc_allocator::{Allocator, Box};
        use oxc_span::Atom;

        let emitter = JsEmitter::new();
        let alloc = Allocator::default();

        let stmt = OutputStatement::DeclareFunction(Box::new_in(
            DeclareFunctionStmt {
                name: Atom::from("foo"),
                params: oxc_allocator::Vec::new_in(&alloc),
                statements: oxc_allocator::Vec::new_in(&alloc),
                modifiers: StmtModifier::NONE,
                source_span: None,
            },
            &alloc,
        ));

        let output = emitter.emit_statement(&stmt);
        assert_eq!(output, "function foo() {\n}");
    }
}

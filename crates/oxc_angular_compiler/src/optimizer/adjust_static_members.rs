//! Adjust Angular static members for tree-shaking.
//!
//! This transformation wraps Angular class definitions and their static members
//! in pure IIFEs to enable tree-shaking. Without this, bundlers cannot remove
//! unused Angular components because static member assignments are seen as
//! potential side effects.
//!
//! ## Transformation
//!
//! Before:
//! ```javascript
//! let MyComponent = class MyComponent {};
//! MyComponent.ɵcmp = /* @__PURE__ */ ɵɵdefineComponent({...});
//! MyComponent.ɵfac = (t) => new (t || MyComponent)();
//! ```
//!
//! After:
//! ```javascript
//! let MyComponent = /* @__PURE__ */ (() => {
//!     let MyComponent = class MyComponent {};
//!     MyComponent.ɵcmp = /* @__PURE__ */ ɵɵdefineComponent({...});
//!     MyComponent.ɵfac = (t) => new (t || MyComponent)();
//!     return MyComponent;
//! })();
//! ```
//!
//! ## Static Members
//!
//! Angular static fields that trigger wrapping:
//! - `ɵcmp` - Component definition
//! - `ɵdir` - Directive definition
//! - `ɵfac` - Factory function
//! - `ɵinj` - Injector definition
//! - `ɵmod` - NgModule definition
//! - `ɵpipe` - Pipe definition
//! - `ɵprov` - Provider definition
//! - `INJECTOR_KEY` - Injector key
//!
//! Static members that should be elided (not wrapped):
//! - `ctorParameters` - Constructor parameters metadata
//! - `decorators` - Class decorators metadata
//! - `propDecorators` - Property decorators metadata

use oxc_ast::ast::{
    AssignmentTarget, BindingPattern, Expression, Program, Statement, VariableDeclarationKind,
};
use oxc_span::Span;

use super::Edit;

/// Angular static fields that should trigger wrapping in a pure IIFE.
const ANGULAR_STATICS_TO_WRAP: &[&str] = &[
    "ɵcmp",         // Component definition
    "ɵdir",         // Directive definition
    "ɵfac",         // Factory function
    "ɵinj",         // Injector definition
    "ɵmod",         // NgModule definition
    "ɵpipe",        // Pipe definition
    "ɵprov",        // Provider definition
    "INJECTOR_KEY", // Injector key
];

/// Angular static fields that should be completely elided.
#[allow(dead_code)]
const ANGULAR_STATICS_TO_ELIDE: &[&str] = &[
    "ctorParameters", // Constructor parameters metadata
    "decorators",     // Class decorators metadata
    "propDecorators", // Property decorators metadata
];

/// Transformer that wraps Angular classes and their static members in pure IIFEs.
pub struct AdjustStaticMembersTransformer;

impl AdjustStaticMembersTransformer {
    /// Create a new static members transformer.
    pub fn new() -> Self {
        Self
    }

    /// Transform the program by collecting edits to wrap classes.
    pub fn transform(&self, program: &Program<'_>, source: &str) -> Vec<Edit> {
        let mut edits = Vec::new();

        // Collect class declarations and their static member assignments
        let class_groups = self.find_class_groups(program, source);

        for group in class_groups {
            if let Some(edit_group) = self.wrap_class_group(&group, source) {
                edits.extend(edit_group);
            }
        }

        edits
    }

    /// Find groups of class declarations with their subsequent static member assignments.
    fn find_class_groups<'a>(
        &self,
        program: &'a Program<'a>,
        _source: &str,
    ) -> Vec<ClassGroup<'a>> {
        let mut groups = Vec::new();
        let mut current_group: Option<ClassGroup<'a>> = None;
        let statements = &program.body;

        for (i, stmt) in statements.iter().enumerate() {
            // Check if this is a class declaration (let/const/var X = class X {})
            if let Some((class_name, is_let, stmt_span)) = self.get_class_declaration_info(stmt) {
                // Finalize previous group if any
                if let Some(group) = current_group.take() {
                    if !group.static_members.is_empty() {
                        groups.push(group);
                    }
                }

                // Start a new group
                current_group = Some(ClassGroup {
                    class_name,
                    class_stmt_index: i,
                    class_stmt_span: stmt_span,
                    is_let_or_const: is_let,
                    static_members: Vec::new(),
                    end_stmt_span: stmt_span,
                });
            }
            // Check if this is a static member assignment for the current class
            else if let Some(ref mut group) = current_group {
                if let Some((static_member, expr_stmt_span)) =
                    self.get_static_member_assignment(stmt, group.class_name)
                {
                    if ANGULAR_STATICS_TO_WRAP.contains(&static_member) {
                        group.static_members.push(expr_stmt_span);
                        group.end_stmt_span = expr_stmt_span;
                    }
                } else {
                    // Non-static-member statement breaks the chain
                    // Finalize current group if it has static members
                    if !group.static_members.is_empty() {
                        groups.push(current_group.take().unwrap());
                    }
                    current_group = None;
                }
            }
        }

        // Finalize last group
        if let Some(group) = current_group {
            if !group.static_members.is_empty() {
                groups.push(group);
            }
        }

        groups
    }

    /// Get class declaration info (name, whether it's let/const, and span).
    fn get_class_declaration_info<'a>(
        &self,
        stmt: &'a Statement<'a>,
    ) -> Option<(&'a str, bool, Span)> {
        if let Statement::VariableDeclaration(var_decl) = stmt {
            // Must be let or const (not var)
            let is_let_or_const = matches!(
                var_decl.kind,
                VariableDeclarationKind::Let | VariableDeclarationKind::Const
            );

            // Check for pattern: let X = class X {}
            if let Some(decl) = var_decl.declarations.first() {
                if let Some(init) = &decl.init {
                    if let Expression::ClassExpression(_class) = init {
                        // Get the variable name from the binding pattern
                        if let BindingPattern::BindingIdentifier(ident) = &decl.id {
                            return Some((ident.name.as_str(), is_let_or_const, var_decl.span));
                        }
                    }
                }
            }
        }

        // Also handle class declarations: class X {}
        if let Statement::ClassDeclaration(class) = stmt {
            if let Some(ident) = &class.id {
                return Some((ident.name.as_str(), true, class.span));
            }
        }

        None
    }

    /// Get the static member name if this is a static member assignment for the given class.
    fn get_static_member_assignment<'a>(
        &self,
        stmt: &'a Statement<'a>,
        class_name: &str,
    ) -> Option<(&'a str, Span)> {
        if let Statement::ExpressionStatement(expr_stmt) = stmt {
            if let Expression::AssignmentExpression(assign) = &expr_stmt.expression {
                // Check for: ClassName.staticMember = ...
                if let AssignmentTarget::AssignmentTargetIdentifier(_) = &assign.left {
                    // This is a simple identifier, not a member expression
                    return None;
                }

                if let AssignmentTarget::StaticMemberExpression(member) = &assign.left {
                    // Check if the object is the class name
                    if let Expression::Identifier(obj_ident) = &member.object {
                        if obj_ident.name.as_str() == class_name {
                            return Some((member.property.name.as_str(), expr_stmt.span));
                        }
                    }
                }
            }
        }

        None
    }

    /// Wrap a class group in a pure IIFE.
    fn wrap_class_group(&self, group: &ClassGroup<'_>, source: &str) -> Option<Vec<Edit>> {
        if group.static_members.is_empty() {
            return None;
        }

        let mut edits = Vec::new();

        // Get the range of statements to wrap
        let start_span = group.class_stmt_span;
        let end_span = group.end_stmt_span;

        // Determine if we need to extract the variable declaration part
        // Pattern: let X = class X {} -> we wrap the class expression and assignments
        let (var_prefix, class_expr_start, class_expr_end) =
            self.extract_var_declaration_parts(start_span, source, group.class_name)?;

        // Build the wrapped code
        let mut wrapped = String::new();

        // Start with the variable declaration prefix if present
        if !var_prefix.is_empty() {
            wrapped.push_str(&var_prefix);
        }

        // Add IIFE header
        wrapped.push_str("/* @__PURE__ */ (() => {\n");

        // Add the inner class declaration (re-declare with let inside IIFE)
        wrapped.push_str("  let ");
        wrapped.push_str(group.class_name);
        wrapped.push_str(" = ");
        let class_body_start = class_expr_start as usize;
        let class_body_end = class_expr_end as usize;
        wrapped.push_str(&source[class_body_start..class_body_end]);
        wrapped.push_str(";\n");

        // Add the static member assignments
        for static_span in &group.static_members {
            let stmt_start = static_span.start as usize;
            let stmt_end = static_span.end as usize;
            wrapped.push_str("  ");
            wrapped.push_str(&source[stmt_start..stmt_end]);
            wrapped.push('\n');
        }

        // Add return statement and close IIFE
        wrapped.push_str("  return ");
        wrapped.push_str(group.class_name);
        wrapped.push_str(";\n})()");

        // Check if original ended with semicolon
        let original_end = end_span.end as usize;
        if original_end < source.len() && source.as_bytes().get(original_end - 1) == Some(&b';') {
            wrapped.push(';');
        } else {
            wrapped.push(';');
        }

        // Create edit to replace the entire group
        edits.push(Edit::replace(start_span.start, end_span.end, wrapped));

        Some(edits)
    }

    /// Extract variable declaration parts from a class statement span.
    /// Returns (var_prefix like "let X = ", class_expr_start, class_expr_end)
    fn extract_var_declaration_parts(
        &self,
        stmt_span: Span,
        source: &str,
        class_name: &str,
    ) -> Option<(String, u32, u32)> {
        let stmt_text = &source[stmt_span.start as usize..stmt_span.end as usize];

        // Check if it starts with let/const/var
        if let Some(eq_pos) = stmt_text.find('=') {
            // Find the variable declaration keyword
            let prefix = stmt_text[..eq_pos].trim();

            // Split prefix into keyword and name
            if let Some(first_space) = prefix.find(|c: char| c.is_whitespace()) {
                let keyword = prefix[..first_space].trim();
                let var_name = prefix[first_space..].trim();

                // Validate it's a variable declaration
                if matches!(keyword, "let" | "const" | "var") {
                    // Find where the class expression starts (after "= ")
                    let class_start = stmt_span.start + eq_pos as u32 + 1;
                    // Skip whitespace after =
                    let mut actual_start = class_start;
                    while (actual_start as usize) < source.len()
                        && source.as_bytes()[actual_start as usize].is_ascii_whitespace()
                    {
                        actual_start += 1;
                    }

                    // Find where it ends (before semicolon if present)
                    let mut class_end = stmt_span.end;
                    if source.as_bytes().get((class_end - 1) as usize) == Some(&b';') {
                        class_end -= 1;
                    }

                    let var_prefix = format!("{} {} = ", keyword, var_name);
                    return Some((var_prefix, actual_start, class_end));
                }
            }
        } else {
            // It's a class declaration: class X {}
            // We must assign the IIFE result to a variable so the class name
            // remains in scope for subsequent export statements.
            let var_prefix = format!("let {} = ", class_name);
            return Some((var_prefix, stmt_span.start, stmt_span.end));
        }

        None
    }
}

impl Default for AdjustStaticMembersTransformer {
    fn default() -> Self {
        Self::new()
    }
}

/// A group of a class declaration and its associated static member assignments.
struct ClassGroup<'a> {
    /// The name of the class.
    class_name: &'a str,
    /// Index of the class statement in the program body.
    #[allow(dead_code)]
    class_stmt_index: usize,
    /// Span of the class declaration statement.
    class_stmt_span: Span,
    /// Whether the class is declared with let or const (vs var or class declaration).
    #[allow(dead_code)]
    is_let_or_const: bool,
    /// Spans of static member assignment statements that follow the class.
    static_members: Vec<Span>,
    /// Span of the last statement in this group.
    end_stmt_span: Span,
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
        let transformer = AdjustStaticMembersTransformer::new();
        let edits = transformer.transform(&result.program, code);
        apply_edits(code, edits)
    }

    #[test]
    fn test_wrap_class_with_static_members() {
        let code = r#"let MyComponent = class MyComponent {};
MyComponent.ɵcmp = defineComponent({});
MyComponent.ɵfac = (t) => new (t || MyComponent)();"#;

        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ (() => {"));
        assert!(result.contains("return MyComponent;"));
        assert!(result.contains("})()"));
    }

    #[test]
    fn test_no_wrap_without_angular_statics() {
        let code = r#"let MyClass = class MyClass {};
MyClass.someOtherStatic = 123;"#;

        let result = transform(code);
        // Should not wrap because `someOtherStatic` is not an Angular static
        assert!(!result.contains("/* @__PURE__ */"));
    }

    #[test]
    fn test_wrap_directive() {
        let code = r#"let MyDirective = class MyDirective {};
MyDirective.ɵdir = defineDirective({});
MyDirective.ɵfac = (t) => new (t || MyDirective)();"#;

        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ (() => {"));
    }

    #[test]
    fn test_wrap_pipe() {
        let code = r#"let MyPipe = class MyPipe {};
MyPipe.ɵpipe = definePipe({});
MyPipe.ɵfac = (t) => new (t || MyPipe)();"#;

        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ (() => {"));
    }

    #[test]
    fn test_wrap_injectable() {
        let code = r#"let MyService = class MyService {};
MyService.ɵprov = defineProvider({});
MyService.ɵfac = (t) => new (t || MyService)();"#;

        let result = transform(code);
        assert!(result.contains("/* @__PURE__ */ (() => {"));
    }
}

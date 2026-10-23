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
    AssignmentTarget, BindingPattern, Class, ClassElement, Declaration, Expression,
    ObjectPropertyKind, Program, PropertyKey, PropertyKind, Statement, VariableDeclarationKind,
};
use oxc_span::{GetSpan, Span};
use rustc_hash::FxHashSet;

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

/// How a class is declared at the top level
#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclarationKind {
    /// `let X = class X {}` (or const/var): wrapped as `<keyword> X = (() => { let X = class X {}; …; return X; })()`
    Variable(&'static str),
    /// `class X {}`, or `export class X {}`: wrapped as `[export] let X = (() => { class X {} …; return X; })()`
    Declaration,
}

/// A class declaration and the static member assignments that follow it.
struct ClassGroup<'a> {
    class_name: &'a str,
    kind: DeclarationKind,
    class: &'a Class<'a>,
    /// Where the wrapper starts: the statement for a variable, the class itself for a (possibly exported) declaration
    start: u32,
    /// Spans of the Angular static member assignments that follow the class.
    static_members: Vec<Span>,
    /// End of the last statement in this group.
    end: u32,
    /// A later statement assigns a member of the class with possible side effects: wrapping would not let it go
    side_effects: bool,
}

impl AdjustStaticMembersTransformer {
    /// Create a new static members transformer.
    pub fn new() -> Self {
        Self
    }

    /// Transform the program by collecting edits to wrap classes.
    pub fn transform(&self, program: &Program<'_>, source: &str) -> Vec<Edit> {
        let bindings = top_level_bindings(program);
        let mut edits = Vec::new();
        for group in self.find_class_groups(program, &bindings, source) {
            // decorators run when the class is defined, and `@dec export let X = …` would not parse
            if group.side_effects || !group.class.decorators.is_empty() {
                continue;
            }
            let Some(members_wrap) = class_static_members_wrappable(group.class, &bindings, source)
            else {
                continue;
            };
            // Like the Angular CLI's adjust-static-class-members: downleveled Angular statics after the class, or
            // static members inside it that are all safe to wrap
            if !group.static_members.is_empty() || members_wrap {
                edits.extend(self.wrap_class_group(&group, source));
            }
        }
        edits
    }

    /// Find groups of class declarations with their subsequent static member assignments.
    fn find_class_groups<'a>(
        &self,
        program: &'a Program<'a>,
        bindings: &FxHashSet<&str>,
        source: &str,
    ) -> Vec<ClassGroup<'a>> {
        let mut groups: Vec<ClassGroup<'a>> = Vec::new();
        let mut current: Option<ClassGroup<'a>> = None;

        for stmt in &program.body {
            if let Some(group) = self.get_class_declaration_info(stmt) {
                if let Some(done) = current.take() {
                    groups.push(done);
                }
                current = Some(group);
                continue;
            }
            let Some(group) = current.as_mut() else { continue };
            // the CLI's analyzeClassSiblings: Angular statics and pure values join the class; any other value
            // assigned to it is a possible side effect, and another kind of statement ends the group
            match self.get_static_member_assignment(stmt, group.class_name) {
                Some((member, span, _)) if ANGULAR_STATICS_TO_WRAP.contains(&member) => {
                    group.static_members.push(span);
                    group.end = span.end;
                }
                Some((member, span, value))
                    if can_wrap_property(member, value, bindings, source) =>
                {
                    group.end = span.end;
                }
                Some(_) => {
                    group.side_effects = true;
                    groups.extend(current.take());
                }
                None => groups.extend(current.take()),
            }
        }
        groups.extend(current);
        groups
    }

    /// The class a top-level statement declares, if any
    fn get_class_declaration_info<'a>(&self, stmt: &'a Statement<'a>) -> Option<ClassGroup<'a>> {
        let group = |class_name, kind, class, start, end| ClassGroup {
            class_name,
            kind,
            class,
            start,
            static_members: Vec::new(),
            end,
            side_effects: false,
        };
        match stmt {
            Statement::VariableDeclaration(var_decl) if var_decl.declarations.len() == 1 => {
                let decl = &var_decl.declarations[0];
                let (
                    BindingPattern::BindingIdentifier(ident),
                    Some(Expression::ClassExpression(class)),
                ) = (&decl.id, &decl.init)
                else {
                    return None;
                };
                let keyword = match var_decl.kind {
                    VariableDeclarationKind::Let => "let",
                    VariableDeclarationKind::Const => "const",
                    VariableDeclarationKind::Var => "var",
                    _ => return None,
                };
                Some(group(
                    ident.name.as_str(),
                    DeclarationKind::Variable(keyword),
                    &**class,
                    var_decl.span.start,
                    var_decl.span.end,
                ))
            }
            Statement::ClassDeclaration(class) => {
                let ident = class.id.as_ref()?;
                Some(group(
                    ident.name.as_str(),
                    DeclarationKind::Declaration,
                    &**class,
                    class.span.start,
                    class.span.end,
                ))
            }
            Statement::ExportDeclaration(export) => {
                let Declaration::ClassDeclaration(class) = &export.declaration else { return None };
                let ident = class.id.as_ref()?;
                Some(group(
                    ident.name.as_str(),
                    DeclarationKind::Declaration,
                    &**class,
                    class.span.start,
                    class.span.end,
                ))
            }
            _ => None,
        }
    }

    /// Get the static member name if this is a static member assignment for the given class.
    fn get_static_member_assignment<'a>(
        &self,
        stmt: &'a Statement<'a>,
        class_name: &str,
    ) -> Option<(&'a str, Span, &'a Expression<'a>)> {
        let Statement::ExpressionStatement(expr_stmt) = stmt else { return None };
        let Expression::AssignmentExpression(assign) = &expr_stmt.expression else { return None };
        let AssignmentTarget::StaticMemberExpression(member) = &assign.left else { return None };
        match &member.object {
            Expression::Identifier(obj) if obj.name.as_str() == class_name => {
                Some((member.property.name.as_str(), expr_stmt.span, &assign.right))
            }
            _ => None,
        }
    }

    /// Wrap a class group in a pure IIFE, by inserting around it: other passes' edits inside it stay valid
    fn wrap_class_group(&self, group: &ClassGroup<'_>, source: &str) -> Vec<Edit> {
        let name = group.class_name;
        let keyword = match group.kind {
            DeclarationKind::Variable(keyword) => keyword,
            DeclarationKind::Declaration => "let",
        };
        let mut end = group.end;
        // a variable statement keeps its own `;` inside the arrow function
        let tail = if source.as_bytes().get(end as usize - 1) == Some(&b';')
            || group.kind == DeclarationKind::Declaration
        {
            format!("\n  return {name};\n}})();")
        } else {
            format!(";\n  return {name};\n}})();")
        };
        // nothing after the group on its last line belongs inside
        if end as usize > source.len() {
            end = source.len() as u32;
        }
        vec![
            Edit::insert(group.start, format!("{keyword} {name} = /* @__PURE__ */ (() => {{\n")),
            Edit { start: end, end, replacement: tail, priority: 1 },
        ]
    }
}

/// Names declared at the top level (imports, functions, classes, variables): an identifier bound there is pure to
/// read, as Babel's `scope.isPure` sees one with a binding
fn top_level_bindings<'a>(program: &'a Program<'a>) -> FxHashSet<&'a str> {
    let mut names = FxHashSet::default();
    let add_pattern = |p: &'a BindingPattern<'a>, names: &mut FxHashSet<&'a str>| {
        if let BindingPattern::BindingIdentifier(id) = p {
            names.insert(id.name.as_str());
        }
    };
    for stmt in &program.body {
        let decl: Option<&Declaration> = match stmt {
            Statement::ImportDeclaration(import) => {
                for spec in import.specifiers.iter().flatten() {
                    names.insert(spec.local().name.as_str());
                }
                None
            }
            Statement::ExportDeclaration(export) => Some(&export.declaration),
            other => other.as_declaration(),
        };
        match decl {
            Some(Declaration::VariableDeclaration(v)) => {
                for d in &v.declarations {
                    add_pattern(&d.id, &mut names);
                }
            }
            Some(Declaration::FunctionDeclaration(f)) => {
                if let Some(id) = &f.id {
                    names.insert(id.name.as_str());
                }
            }
            Some(Declaration::ClassDeclaration(c)) => {
                if let Some(id) = &c.id {
                    names.insert(id.name.as_str());
                }
            }
            _ => {}
        }
    }
    names
}

/// The Angular CLI's `analyzeClassStaticProperties`, run for every class: `None` when defining the class can have
/// side effects, else whether its static members alone make it worth wrapping. Every static property, and every nonempty static block, is
/// `this.<name> = …` holding an Angular definition or a value without side effects. Private and computed static
/// fields and other static blocks are not wrapped. Unlike the CLI, an impure superclass or computed key also keeps
/// the class out: both run when the class is defined, and the pure wrapper would let them be dropped.
fn class_static_members_wrappable(
    class: &Class<'_>,
    bindings: &FxHashSet<&str>,
    source: &str,
) -> Option<bool> {
    if class.heritage.as_ref().is_some_and(|h| {
        !has_pure_annotation(h.expression.span(), source)
            && !is_pure_reference(&h.expression, bindings)
    }) {
        return None;
    }
    let mut should_wrap = false;
    for element in &class.body.body {
        if element.computed()
            && element
                .property_key()
                .and_then(PropertyKey::as_expression)
                .is_none_or(|key| !is_pure_reference(key, bindings))
        {
            return None;
        }
        match element {
            ClassElement::PropertyDefinition(prop) if prop.r#static => {
                let PropertyKey::StaticIdentifier(key) = &prop.key else { return None };
                match &prop.value {
                    None => should_wrap = true,
                    Some(value)
                        if can_wrap_property(key.name.as_str(), value, bindings, source) =>
                    {
                        should_wrap = true
                    }
                    Some(_) => return None,
                }
            }
            ClassElement::StaticBlock(block) => {
                let stmt = match block.body.as_slice() {
                    [] => continue,
                    [Statement::ExpressionStatement(stmt)] => stmt,
                    _ => return None,
                };
                let Expression::AssignmentExpression(assign) = &stmt.expression else {
                    return None;
                };
                let AssignmentTarget::StaticMemberExpression(member) = &assign.left else {
                    return None;
                };
                if !matches!(member.object, Expression::ThisExpression(_)) {
                    return None;
                }
                if can_wrap_property(member.property.name.as_str(), &assign.right, bindings, source)
                {
                    should_wrap = true;
                } else {
                    return None;
                }
            }
            // `static accessor x = …` runs its initializer when the class is defined, and the CLI does not check it
            ClassElement::AccessorProperty(accessor)
                if accessor.r#static && accessor.value.is_some() =>
            {
                return None;
            }
            _ => {}
        }
    }
    Some(should_wrap)
}

/// The Angular CLI's `canWrapProperty`: an Angular definition, a value marked pure, or a pure expression
fn can_wrap_property(
    name: &str,
    value: &Expression<'_>,
    bindings: &FxHashSet<&str>,
    source: &str,
) -> bool {
    ANGULAR_STATICS_TO_WRAP.contains(&name)
        || has_pure_annotation(value.span(), source)
        || is_pure(value, bindings)
}

/// A `/* @__PURE__ */`, `/* #__PURE__ */` or `@pureOrBreakMyCode` comment right before the expression
fn has_pure_annotation(span: Span, source: &str) -> bool {
    let before = source[..span.start as usize].trim_end();
    let Some(body) = before.strip_suffix("*/") else { return false };
    let Some(open) = body.rfind("/*") else { return false };
    let comment = &body[open..];
    comment.contains("@__PURE__")
        || comment.contains("#__PURE__")
        || comment.contains("@pureOrBreakMyCode")
}

/// A superclass or computed key the CLI would not check at all: a pure value, or a member read on one such as
/// `i0.Base` or `Symbol.iterator`. A member read could run a getter, so static values do not accept it.
fn is_pure_reference(expr: &Expression<'_>, bindings: &FxHashSet<&str>) -> bool {
    match expr {
        Expression::StaticMemberExpression(m) => is_pure_reference(&m.object, bindings),
        Expression::ParenthesizedExpression(p) => is_pure_reference(&p.expression, bindings),
        _ => is_pure(expr, bindings),
    }
}

/// Globals that can be read without side effects
const PURE_GLOBALS: &[&str] = &["undefined", "NaN", "Infinity", "Symbol"];

/// Babel's `scope.isPure`, for what static members hold: literals, functions, classes, bound identifiers, and
/// arrays, objects, templates and operators made of those
fn is_pure(expr: &Expression<'_>, bindings: &FxHashSet<&str>) -> bool {
    match expr {
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::FunctionExpression(_)
        | Expression::ArrowFunctionExpression(_) => true,
        Expression::Identifier(id) => {
            bindings.contains(id.name.as_str()) || PURE_GLOBALS.contains(&id.name.as_str())
        }
        Expression::LogicalExpression(l) => {
            is_pure(&l.left, bindings) && is_pure(&l.right, bindings)
        }
        Expression::TemplateLiteral(t) => t.expressions.iter().all(|e| is_pure(e, bindings)),
        Expression::ArrayExpression(a) => a.elements.iter().all(|el| match el {
            oxc_ast::ast::ArrayExpressionElement::SpreadElement(_) => false,
            oxc_ast::ast::ArrayExpressionElement::Elision(_) => true,
            other => other.as_expression().is_some_and(|e| is_pure(e, bindings)),
        }),
        Expression::ObjectExpression(o) => o.properties.iter().all(|p| match p {
            ObjectPropertyKind::SpreadProperty(_) => false,
            ObjectPropertyKind::ObjectProperty(prop) => {
                let key_pure = !prop.computed
                    || prop.key.as_expression().is_some_and(|k| is_pure(k, bindings));
                key_pure
                    && (prop.method || matches!(prop.kind, PropertyKind::Init))
                    && (prop.method || is_pure(&prop.value, bindings))
            }
        }),
        Expression::UnaryExpression(u) => is_pure(&u.argument, bindings),
        Expression::BinaryExpression(b) => {
            is_pure(&b.left, bindings) && is_pure(&b.right, bindings)
        }
        Expression::ParenthesizedExpression(p) => is_pure(&p.expression, bindings),
        Expression::ClassExpression(c) => {
            c.heritage.as_ref().is_none_or(|h| is_pure(&h.expression, bindings))
                && c.body.body.iter().all(|el| match el {
                    ClassElement::MethodDefinition(m) => !m.computed,
                    ClassElement::PropertyDefinition(p) => {
                        !p.computed && p.value.as_ref().is_none_or(|v| is_pure(v, bindings))
                    }
                    _ => false,
                })
        }
        _ => false,
    }
}

impl Default for AdjustStaticMembersTransformer {
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

    /// The linker's output form: Angular definitions in static blocks inside the class
    #[test]
    fn wraps_class_with_angular_definitions_in_static_blocks() {
        let code = r#"class ButtonComponent {
    constructor() { this.size = 1; }
    static { this.ɵfac = function ButtonComponent_Factory(t) { return new (t || ButtonComponent)(); }; }
    static { this.ɵcmp = (() => { return i0.ɵɵdefineComponent({ type: ButtonComponent }); })(); }
}
export { ButtonComponent };"#;
        let result = transform(code);
        assert!(
            result.starts_with(
                "let ButtonComponent = /* @__PURE__ */ (() => {\nclass ButtonComponent {"
            ),
            "{result}"
        );
        assert!(
            result.contains("return ButtonComponent;\n})();\nexport { ButtonComponent };"),
            "{result}"
        );
    }

    #[test]
    fn wraps_exported_class_declarations() {
        let code =
            "export class A {\n    static { this.ɵprov = i0.ɵɵdefineInjectable({ token: A }); }\n}";
        let result = transform(code);
        assert_eq!(
            result,
            "export let A = /* @__PURE__ */ (() => {\nclass A {\n    static { this.ɵprov = i0.ɵɵdefineInjectable({ token: A }); }\n}\n  return A;\n})();"
        );
    }

    /// Deepkit's runtime type: an array of strings and functions
    #[test]
    fn wraps_pure_static_values() {
        let code = r#"class T {
    static { this.__type = ['x', function () { return 1; }, () => T, 'P"0!']; }
    static count = 0;
}"#;
        assert!(transform(code).contains("/* @__PURE__ */ (() => {"));
    }

    #[test]
    fn keeps_static_members_with_side_effects() {
        for code in [
            "class A {\n    static { this.x = register(A); }\n}",
            "class A {\n    static { register(A); this.ɵfac = f; }\n}",
            "class A {\n    static { window.a = 1; }\n}",
            "class A {\n    static x = sideEffect();\n}",
            "class A {\n    static [key] = 1;\n}",
            "class A {\n    static x = undeclaredGlobal;\n}",
        ] {
            assert_eq!(transform(code), code, "must not wrap: {code}");
        }
    }

    #[test]
    fn marked_pure_static_values_wrap() {
        let code = "class A {\n    static x = /* @__PURE__ */ make();\n}";
        assert!(transform(code).contains("/* @__PURE__ */ (() => {"));
    }

    /// A class whose body has an `=` (a constructor assignment) with downleveled statics after it
    #[test]
    fn wraps_class_declaration_with_assignments_inside() {
        let code = "class A {\n    constructor() { this.a = 1; }\n}\nA.ɵfac = function (t) { return new (t || A)(); };\nA.ɵcmp = defineComponent({});";
        let result = transform(code);
        assert!(result.starts_with("let A = /* @__PURE__ */ (() => {\nclass A {"), "{result}");
        assert!(result.ends_with("A.ɵcmp = defineComponent({});\n  return A;\n})();"), "{result}");
    }

    #[test]
    fn class_definition_side_effects_stay() {
        // each of these runs code when the class is defined, so a pure wrapper could drop it
        for code in [
            "class A { static x = 0; static #r = register(); }",
            "class A { static x = 0; static { register(A); } }",
            "let y; class A { static x = 0; static { y = register(); } }",
            "class A { static x = 0; static { this[key()] = 1; } }",
            "class A extends registerBase() { static x = 0; }",
            "class A { static x = 0; [register()]() {} }",
            "class A { static x = 0; [register()] = 1; }",
            "class A { static x = 0; static accessor y = register(); }",
            // the same with a downleveled Angular static after the class
            "class A extends registerBase() {}\nA.ɵfac = f;",
            "class A {\n    static x = register();\n}\nA.ɵfac = f;",
            "class A {\n    static { register(A); }\n}\nA.ɵfac = f;",
            // a member read could run a getter
            "import { registry } from 'x';\nclass A { static x = registry.value; }",
        ] {
            assert_eq!(transform(code), code, "{code}");
        }
        // decorated classes stay too: `@dec export let A = …` would not parse
        let code = "@dec export class A {\n    static ɵfac = f;\n}";
        assert_eq!(transform(code), code);
        // pure superclasses, computed keys and values still wrap
        for code in [
            "class B {}\nclass A extends B { static x = 0; ['k']() {} }",
            "import * as i0 from 'x';\nclass A extends i0.Base { static x = 0; *[Symbol.iterator]() {} }",
            "class A extends /* @__PURE__ */ mixin() { static x = 0; }",
            "let a, b;\nclass A { static x = a || b; static y = undefined; }",
        ] {
            assert!(transform(code).contains("/* @__PURE__ */"), "{code}");
        }
    }

    #[test]
    fn plain_classes_stay() {
        let code = "class A {\n    method() { return 1; }\n}";
        assert_eq!(transform(code), code);
    }
}

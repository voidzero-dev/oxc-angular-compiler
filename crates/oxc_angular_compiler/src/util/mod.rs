//! Shared utilities for the Angular compiler.

pub mod chars;
mod deferred_time;
pub(crate) mod lone_surrogate;
mod parse_util;
mod type_extract;

pub use deferred_time::*;
pub use parse_util::*;
pub use type_extract::*;

/// Whether a property of a decorator's options object is read as metadata.
/// Like ngtsc's `reflectObjectLiteral`, methods and accessors are not.
pub fn is_metadata_property(prop: &oxc_ast::ast::ObjectProperty<'_>) -> bool {
    !prop.method && prop.kind == oxc_ast::ast::PropertyKind::Init
}

/// The class's constructor implementation: the one with a body. Overload
/// signatures (no body) are skipped, since the implementation is what runs and
/// what carries the parameter decorators.
///
/// See: packages/compiler-cli/src/ngtsc/reflection/src/typescript.ts
/// (`TypeScriptReflectionHost.getConstructorParameters`)
pub fn find_constructor<'c, 'a>(
    class: &'c oxc_ast::ast::Class<'a>,
) -> Option<&'c oxc_ast::ast::MethodDefinition<'a>> {
    class.body.body.iter().find_map(|element| match element {
        oxc_ast::ast::ClassElement::MethodDefinition(method)
            if method.kind.is_constructor() && method.value.body.is_some() =>
        {
            Some(&**method)
        }
        _ => None,
    })
}

/// A class declaration of a file, with the top-level statement it is in.
pub struct DeclaredClass<'p, 'a> {
    /// The declaration.
    pub class: &'p oxc_ast::ast::Class<'a>,
    /// The class's own declaration (`export` / `export default` included) for
    /// a top-level class; for a nested one, the top-level statement holding
    /// the function or block it is declared in.
    pub top_level: &'p oxc_ast::ast::Statement<'a>,
    /// Declared in a function body or block rather than at the top level.
    /// ngtsc compiles these too, and hoists their constants to the file top.
    pub nested: bool,
}

/// Every class declaration of `program`, top-level and nested alike, in the
/// order ngtsc compiles them: source order, except that the classes declared
/// in a class's members come before it. Classes in a `namespace` and `declare
/// class`es are left out, as are class expressions: TypeScript rejects
/// decorators on those.
pub fn declared_classes<'p, 'a>(
    program: &'p oxc_ast::ast::Program<'a>,
) -> std::vec::Vec<DeclaredClass<'p, 'a>> {
    use oxc_ast::ast::{Class, Declaration, ExportDefaultDeclarationKind, Statement};
    use oxc_ast_visit::Visit;

    struct Nested<'a>(std::vec::Vec<*const Class<'a>>);
    impl<'a> Visit<'a> for Nested<'a> {
        fn visit_class(&mut self, class: &Class<'a>) {
            oxc_ast_visit::walk::walk_class(self, class);
            if class.is_declaration() && !class.declare {
                self.0.push(class);
            }
        }
        fn visit_ts_module_block(&mut self, _: &oxc_ast::ast::TSModuleBlock<'a>) {}
    }

    let mut classes = std::vec::Vec::new();
    for stmt in &program.body {
        let class = match stmt {
            Statement::ClassDeclaration(class) => Some(class),
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::ClassDeclaration(class) => Some(class),
                _ => None,
            },
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::ClassDeclaration(class) => Some(class),
                _ => None,
            },
            _ => None,
        };
        let mut nested = Nested(std::vec::Vec::new());
        match class {
            Some(class) => oxc_ast_visit::walk::walk_class(&mut nested, class),
            None => nested.visit_statement(stmt),
        }
        for class in nested.0 {
            // SAFETY: the visitor saw `class` while walking `stmt`, which is
            // borrowed from `program` for `'p` and not mutated meanwhile.
            let class = unsafe { &*class };
            classes.push(DeclaredClass { class, top_level: stmt, nested: true });
        }
        if let Some(class) = class {
            classes.push(DeclaredClass { class, top_level: stmt, nested: false });
        }
    }
    classes
}

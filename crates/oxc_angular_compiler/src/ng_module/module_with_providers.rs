//! `ModuleWithProviders` entries in a partial `ɵɵngDeclareNgModule`'s
//! `imports`.
//!
//! ngtsc evaluates each `imports` entry, and an entry that gives a
//! `ModuleWithProviders` (`Dep.forRoot()`, or a constant holding one) is
//! listed as the NgModule it names, not as written: `imports: [mwp,
//! Holder.forRoot()]` becomes `imports: [DepModule, DepModule]`. See
//! `NgModuleDecoratorHandler.resolveTypeList` and
//! `createModuleWithProvidersResolver` in
//! `packages/compiler-cli/src/ngtsc/annotations/ng_module/src/`.
//!
//! Oxc used to list the class a method is called on (`Holder`), which is
//! not an NgModule at all when another class holds the method. When a JIT
//! linker inlines that list into `ɵɵdefineNgModule`, the runtime gets a
//! class that is no module and a `ModuleWithProviders` object.
//!
//! Only the partial declaration changes: the injector keeps the raw
//! entries (it needs their providers), and the full-mode
//! `ɵɵsetNgModuleScope` keeps the raw list, which the runtime unwraps.
//!
//! This file reads what one file can tell. With cross-file analysis, a
//! method or constant of another file is read by
//! `CrossFileAnalyzer::module_with_providers_imports`.

use oxc_ast::ast::{
    ArrayExpressionElement, Class, ClassElement, Declaration, Expression, MethodDefinitionKind,
    ObjectPropertyKind, Program, PropertyKey, Statement, TSType, TSTypeName,
};

use crate::directive::StringConsts;

use super::decorator::find_ng_module_decorator;

/// Where the NgModule of a `ModuleWithProviders` is written.
pub enum MwpModule<'b, 'a> {
    /// The `ngModule` value of a returned object literal.
    Value(&'b Expression<'a>),
    /// `T` of a `ModuleWithProviders<T>` return type.
    Type(&'b TSTypeName<'a>),
    /// `return Class.method(...)` (or `ns.Class.method(...)`): what that
    /// call gives. When ngtsc can't evaluate it either, it reads this
    /// method's return type instead: `fallback`, `None` when unsure.
    Call {
        namespace: Option<&'b str>,
        class: &'b str,
        method: &'b str,
        fallback: Option<MwpType<'b, 'a>>,
    },
    /// ngtsc can't evaluate the method, and its return type names no
    /// module. A method returning a call of it falls back to its own type.
    Dynamic,
}

/// What ngtsc reads from a method's return type.
#[derive(Clone, Copy)]
pub enum MwpType<'b, 'a> {
    /// `T` of `ModuleWithProviders<T>`.
    Module(&'b TSTypeName<'a>),
    /// No return type, or one that isn't a `ModuleWithProviders`.
    Dynamic,
}

/// What a call to a `ModuleWithProviders` method gives: a module, a value
/// ngtsc can't evaluate (`Dynamic`), or one Oxc can't tell (`Unknown`).
pub enum MwpRead<T> {
    Known(T),
    Dynamic,
    Unknown,
}

impl<T> MwpRead<T> {
    /// The module, if known.
    pub fn known(self) -> Option<T> {
        match self {
            Self::Known(module) => Some(module),
            Self::Dynamic | Self::Unknown => None,
        }
    }
}

/// The NgModule the static method `method` of `class` returns a
/// `ModuleWithProviders` of, read in ngtsc's order. Its evaluator first
/// runs a body that is a single `return` (`visitFunctionBody`): an object
/// literal with `ngModule` there is the answer, and a call is evaluated the
/// same way. When that gives a dynamic value (any other body, `satisfies`,
/// a getter in the object...), `createModuleWithProvidersResolver` reads
/// the `ModuleWithProviders<T>` return type. `None` when unsure: a
/// parameter or type parameter in place of the module, or a value ngtsc
/// would evaluate further.
///
/// With overloads, ngtsc reads only the first declaration (the symbol's
/// `valueDeclaration`), whatever the call's arguments: a signature has no
/// body, so its return type decides.
pub fn static_method_ng_module<'b, 'a>(
    class: &'b Class<'a>,
    method: &str,
) -> Option<MwpModule<'b, 'a>> {
    let function = &class
        .body
        .body
        .iter()
        .find_map(|element| match element {
            ClassElement::MethodDefinition(m)
                if m.r#static
                    && !m.computed
                    && m.key.static_name().is_some_and(|key| key == method) =>
            {
                Some(m)
            }
            _ => None,
        })
        .filter(|m| m.kind == MethodDefinitionKind::Method)?
        .value;
    let is_param = |expr: &Expression<'_>| {
        // `param`, `param.X`: the value comes from the call's argument.
        let mut root = expr;
        while let Expression::StaticMemberExpression(member) = root {
            root = evaluated(&member.object);
        }
        matches!(root, Expression::Identifier(id) if function.params.items.iter().any(|param| {
            param.pattern.get_binding_identifiers().iter().any(|b| b.name == id.name)
        }))
    };
    let return_type = || return_type_module(class, function);
    if let Some(body) = &function.body
        && let [Statement::ReturnStatement(ret)] = body.statements.as_slice()
    {
        match evaluated(ret.argument.as_ref()?) {
            Expression::ObjectExpression(object) => match object_ng_module(object)? {
                ObjectModule::Value(value) if is_param(value) => return None,
                ObjectModule::Value(value) => return Some(MwpModule::Value(value)),
                ObjectModule::Dynamic => {}
            },
            Expression::CallExpression(call) => {
                let (namespace, class, method) = static_call(call)?;
                if is_param(evaluated(&call.callee)) {
                    return None;
                }
                return Some(MwpModule::Call { namespace, class, method, fallback: return_type() });
            }
            Expression::TSSatisfiesExpression(_)
            | Expression::TSTypeAssertion(_)
            | Expression::TSInstantiationExpression(_) => {}
            _ => return None,
        }
    }
    match return_type()? {
        MwpType::Module(name) => Some(MwpModule::Type(name)),
        MwpType::Dynamic => Some(MwpModule::Dynamic),
    }
}

/// What ngtsc reads from the return type of `function`, a method of
/// `class`; `None` for a type parameter in place of the module.
fn return_type_module<'b, 'a>(
    class: &'b Class<'a>,
    function: &'b oxc_ast::ast::Function<'a>,
) -> Option<MwpType<'b, 'a>> {
    let Some(return_type) = &function.return_type else { return Some(MwpType::Dynamic) };
    let TSType::TSTypeReference(reference) = &return_type.type_annotation else {
        return Some(MwpType::Dynamic);
    };
    let is_mwp = match &reference.type_name {
        TSTypeName::IdentifierReference(id) => id.name == "ModuleWithProviders",
        TSTypeName::QualifiedName(name) => name.right.name == "ModuleWithProviders",
        TSTypeName::ThisExpression(_) => false,
    };
    if !is_mwp {
        return Some(MwpType::Dynamic);
    }
    let [TSType::TSTypeReference(module)] = reference.type_arguments.as_ref()?.params.as_slice()
    else {
        return None;
    };
    if let TSTypeName::IdentifierReference(id) = &module.type_name {
        let is_type_param = [&function.type_parameters, &class.type_parameters]
            .into_iter()
            .flatten()
            .any(|params| params.params.iter().any(|p| p.name.name == id.name));
        if is_type_param {
            return None;
        }
    }
    Some(MwpType::Module(&module.type_name))
}

/// `expr` without what ngtsc's evaluator looks through: parentheses, `as`
/// and `!` (`visitExpression`). It can't evaluate `satisfies`, `<T>x` or
/// `x<T>`.
fn evaluated<'b, 'a>(mut expr: &'b Expression<'a>) -> &'b Expression<'a> {
    loop {
        expr = match expr {
            Expression::ParenthesizedExpression(e) => &e.expression,
            Expression::TSAsExpression(e) => &e.expression,
            Expression::TSNonNullExpression(e) => &e.expression,
            _ => return expr,
        };
    }
}

/// `(namespace, class, method)` of a `Class.method(...)` or
/// `ns.Class.method(...)` call.
fn static_call<'b>(
    call: &'b oxc_ast::ast::CallExpression<'_>,
) -> Option<(Option<&'b str>, &'b str, &'b str)> {
    let Expression::StaticMemberExpression(member) = evaluated(&call.callee) else { return None };
    let method = member.property.name.as_str();
    match evaluated(&member.object) {
        Expression::Identifier(class) => Some((None, class.name.as_str(), method)),
        Expression::StaticMemberExpression(class) => {
            let Expression::Identifier(ns) = evaluated(&class.object) else { return None };
            Some((Some(ns.name.as_str()), class.property.name.as_str(), method))
        }
        _ => None,
    }
}

/// What ngtsc reads from a returned object literal.
enum ObjectModule<'b, 'a> {
    /// Its `ngModule` value.
    Value(&'b Expression<'a>),
    /// Nothing: a getter or method makes it give up on the object
    /// (`visitObjectLiteralExpression`).
    Dynamic,
}

/// The `ngModule` of an object literal, `None` when unsure: a spread or
/// computed key could set it, or there is none.
fn object_ng_module<'b, 'a>(
    object: &'b oxc_ast::ast::ObjectExpression<'a>,
) -> Option<ObjectModule<'b, 'a>> {
    let mut value = None;
    for property in &object.properties {
        let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
        if property.kind != oxc_ast::ast::PropertyKind::Init || property.method {
            return Some(ObjectModule::Dynamic);
        }
        if property.computed {
            return None;
        }
        if property.key.static_name().is_some_and(|key| key == "ngModule") {
            value = Some(evaluated(&property.value));
        }
    }
    value.map(ObjectModule::Value)
}

/// How a constant's initializer gives a `ModuleWithProviders`.
pub enum MwpInit<'b, 'a> {
    /// `Class.method(...)` or `ns.Class.method(...)`.
    Call { namespace: Option<&'b str>, class: &'b str, method: &'b str },
    /// `{ ngModule: T, ... }`: the `T`.
    Module(&'b Expression<'a>),
}

/// How `init` gives a `ModuleWithProviders`, if it is one of the shapes
/// ngtsc can evaluate to one.
pub fn module_with_providers_init<'b, 'a>(init: &'b Expression<'a>) -> Option<MwpInit<'b, 'a>> {
    match evaluated(init) {
        Expression::CallExpression(call) => {
            let (namespace, class, method) = static_call(call)?;
            Some(MwpInit::Call { namespace, class, method })
        }
        Expression::ObjectExpression(object) => match object_ng_module(object)? {
            ObjectModule::Value(value) => Some(MwpInit::Module(value)),
            ObjectModule::Dynamic => None,
        },
        _ => None,
    }
}

/// The initializer of the top-level `const name = ...` of `program`.
pub fn top_level_const_init<'b, 'a>(
    program: &'b Program<'a>,
    name: &str,
) -> Option<&'b Expression<'a>> {
    program.body.iter().find_map(|stmt| {
        let declaration = match stmt {
            Statement::ExportDeclaration(decl) => &decl.declaration,
            _ => stmt.as_declaration()?,
        };
        let Declaration::VariableDeclaration(vars) = declaration else { return None };
        if !vars.kind.is_const() {
            return None;
        }
        vars.declarations
            .iter()
            .find(|var| var.id.get_binding_identifier().is_some_and(|id| id.name == name))?
            .init
            .as_ref()
    })
}

/// The top-level class `program` declares as `name`.
fn top_level_class<'b, 'a>(program: &'b Program<'a>, name: &str) -> Option<&'b Class<'a>> {
    program.body.iter().find_map(|stmt| {
        let class = match stmt {
            Statement::ClassDeclaration(class) => class,
            Statement::ExportDeclaration(decl) => match &decl.declaration {
                Declaration::ClassDeclaration(class) => class,
                _ => return None,
            },
            Statement::ExportDefaultDeclaration(decl) => match &decl.declaration {
                oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => class,
                _ => return None,
            },
            _ => return None,
        };
        class.id.as_ref().is_some_and(|id| id.name == name).then_some(&**class)
    })
}

/// One `imports` entry, in the order and shape the decorator parser lists
/// them (`extract_reference_array`): its spreads and other expressions are
/// left out.
pub struct ImportEntry<'b> {
    /// The name the partial declaration lists for it today.
    pub name: &'b str,
    /// The `method` of `name.method(...)`, `None` for a plain name or a
    /// `forwardRef`.
    pub method: Option<&'b str>,
    /// Whether it is written as `forwardRef(() => name)`.
    pub forward_ref: bool,
}

/// The `imports` entries of the `@NgModule` of `class`, `None` when its
/// `imports` is not an array literal.
pub fn ng_module_import_entries<'b>(
    class: &'b Class<'_>,
    consts: &StringConsts<'_>,
) -> Option<Vec<ImportEntry<'b>>> {
    let decorator = find_ng_module_decorator(&class.decorators, Some(consts))?;
    let Expression::CallExpression(call) = &decorator.expression else { return None };
    let Some(oxc_ast::ast::Argument::ObjectExpression(config)) = call.arguments.first() else {
        return None;
    };
    // The parser keeps the last `imports` key.
    let imports = config.properties.iter().rev().find_map(|property| {
        let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
        let key = match &property.key {
            PropertyKey::StaticIdentifier(id) => id.name.as_str(),
            PropertyKey::StringLiteral(lit) => lit.value.as_str()?,
            _ => return None,
        };
        (crate::util::is_metadata_property(property) && key == "imports").then_some(&property.value)
    })?;
    let Expression::ArrayExpression(array) = imports else { return None };
    let mut entries = Vec::new();
    for element in &array.elements {
        match element {
            ArrayExpressionElement::Identifier(id) => {
                entries.push(ImportEntry {
                    name: id.name.as_str(),
                    method: None,
                    forward_ref: false,
                });
            }
            ArrayExpressionElement::CallExpression(call) => match &call.callee {
                Expression::Identifier(id) if id.name == "forwardRef" => {
                    if let Some(oxc_ast::ast::Argument::ArrowFunctionExpression(arrow)) =
                        call.arguments.first()
                        && let Some(Expression::Identifier(inner)) = arrow.get_expression()
                    {
                        entries.push(ImportEntry {
                            name: inner.name.as_str(),
                            method: None,
                            forward_ref: true,
                        });
                    }
                }
                Expression::StaticMemberExpression(member) => {
                    if let Expression::Identifier(id) = &member.object {
                        entries.push(ImportEntry {
                            name: id.name.as_str(),
                            method: Some(member.property.name.as_str()),
                            forward_ref: false,
                        });
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    Some(entries)
}

/// The NgModule a `ModuleWithProviders` entry names, when this file alone
/// tells it: the entry is `Class.method(...)` or a top-level `const` of one
/// (or of `{ ngModule: T }`), `Class` is declared in this file, and `T` is
/// a class this file declares before `module_class`, or a binding whose
/// import the output keeps anyway (`is_kept_import`). ngtsc then names `T`
/// as this file writes it (`DepModule`, not `i1.DepModule`).
///
/// A class declared later would be read before it exists, and an import
/// only used as a type is removed, so those stay as they were.
pub fn resolve_local_entry<'b>(
    program: &'b Program<'_>,
    module_class: &Class<'_>,
    entry: &ImportEntry<'_>,
    is_kept_import: impl Fn(&str) -> bool,
) -> Option<&'b str> {
    if entry.forward_ref {
        return None;
    }
    let module = match entry.method {
        Some(method) => method_module(program, entry.name, method)?,
        None => match module_with_providers_init(top_level_const_init(program, entry.name)?)? {
            MwpInit::Call { namespace: None, class, method } => {
                method_module(program, class, method)?
            }
            MwpInit::Call { namespace: Some(_), .. } => return None,
            MwpInit::Module(value) => match value {
                Expression::Identifier(id) => id.name.as_str(),
                _ => return None,
            },
        },
    };
    let declared_before = top_level_class(program, module)
        .is_some_and(|class| class.span.end <= module_class.span.start);
    (declared_before || is_kept_import(module)).then_some(module)
}

/// The name `program` writes the NgModule of `class.method(...)` by, when
/// `class` is declared there.
fn method_module<'b>(program: &'b Program<'_>, class: &str, method: &str) -> Option<&'b str> {
    method_read(program, class, method, 0).known()
}

/// What `class.method(...)` gives, `class` and the methods it calls
/// declared in `program`.
fn method_read<'b>(
    program: &'b Program<'_>,
    class: &str,
    method: &str,
    depth: u8,
) -> MwpRead<&'b str> {
    const MAX_CALLS: u8 = 8;
    let Some(class) = top_level_class(program, class).filter(|_| depth <= MAX_CALLS) else {
        return MwpRead::Unknown;
    };
    let name = |name: &'b TSTypeName<'_>| match name {
        TSTypeName::IdentifierReference(id) => MwpRead::Known(id.name.as_str()),
        _ => MwpRead::Unknown,
    };
    match static_method_ng_module(class, method) {
        Some(MwpModule::Value(Expression::Identifier(id))) => MwpRead::Known(id.name.as_str()),
        Some(MwpModule::Type(type_name)) => name(type_name),
        Some(MwpModule::Dynamic) => MwpRead::Dynamic,
        Some(MwpModule::Call { namespace: None, class, method, fallback }) => {
            match method_read(program, class, method, depth + 1) {
                MwpRead::Dynamic => match fallback {
                    Some(MwpType::Module(type_name)) => name(type_name),
                    Some(MwpType::Dynamic) => MwpRead::Dynamic,
                    None => MwpRead::Unknown,
                },
                read => read,
            }
        }
        _ => MwpRead::Unknown,
    }
}

/// How the transform sees a binding of the compiled file.
pub enum Binding {
    /// A value import the output keeps anyway (`kept`), by name or default.
    Import { kept: bool },
    /// `import * as ns`.
    Namespace,
    /// Anything else: a declaration, a type-only import, nothing.
    Other,
}

/// How the compiled file names a class cross-file analysis found.
#[cfg_attr(not(feature = "cross_file_elision"), expect(dead_code))]
pub enum NamedClass {
    /// A class the file declares, by its name.
    Local(String),
    /// The export `name` of the module `specifier`.
    Module { specifier: String, name: String },
}

/// Cross-file analysis of `(namespace, name, method)` entries: the
/// NgModule each one names, if it can tell.
pub type ResolveImported<'r> =
    &'r dyn Fn(&[(Option<&str>, &str, Option<&str>)]) -> std::vec::Vec<Option<NamedClass>>;

/// The partial `ɵɵngDeclareNgModule` imports of the `@NgModule` `class`
/// with each `ModuleWithProviders` entry listed as its NgModule, or `None`
/// when none was read (or the entries don't line up with `current`, the
/// names listed today). An entry this file can't settle goes to
/// `resolve_imported` (cross-file analysis: `(namespace, name, method)`
/// entries), and an entry no one settles keeps today's name.
#[expect(clippy::too_many_arguments)]
pub fn partial_imports<'a>(
    allocator: &'a oxc_allocator::Allocator,
    program: &Program<'_>,
    class: &Class<'_>,
    consts: &StringConsts<'_>,
    current: &[oxc_str::Ident<'a>],
    binding: impl Fn(&str) -> Binding,
    resolve_imported: Option<ResolveImported<'_>>,
    mut namespace_of: impl FnMut(&str) -> oxc_str::Ident<'a>,
) -> Option<oxc_allocator::Vec<'a, crate::output::ast::OutputExpression<'a>>> {
    use crate::output::ast::{OutputExpression, ReadPropExpr, ReadVarExpr};
    use oxc_allocator::Box;

    let entries = ng_module_import_entries(class, consts)?;
    if entries.len() != current.len()
        || entries.iter().zip(current).any(|(entry, name)| entry.name != name.as_str())
    {
        return None;
    }
    let read_var = |name: oxc_str::Ident<'a>| {
        OutputExpression::ReadVar(Box::new_in(ReadVarExpr { name, source_span: None }, &allocator))
    };
    let mut resolved: std::vec::Vec<Option<OutputExpression<'a>>> = entries
        .iter()
        .map(|entry| {
            let is_kept = |name: &str| matches!(binding(name), Binding::Import { kept: true });
            resolve_local_entry(program, class, entry, is_kept)
                .map(|name| read_var(oxc_str::Ident::from(allocator.alloc_str(name))))
        })
        .collect();

    // The entries whose method or constant may be in another file.
    if let Some(resolve_imported) = resolve_imported {
        let is_import = |name: &str| matches!(binding(name), Binding::Import { .. });
        let mut asked = std::vec::Vec::new();
        let mut questions = std::vec::Vec::new();
        for (index, entry) in entries.iter().enumerate() {
            if resolved[index].is_some() || entry.forward_ref {
                continue;
            }
            let question = match entry.method {
                Some(method) if is_import(entry.name) => (None, entry.name, Some(method)),
                Some(_) => continue,
                None if is_import(entry.name) => (None, entry.name, None),
                None => {
                    let Some(MwpInit::Call { namespace, class, method }) =
                        top_level_const_init(program, entry.name)
                            .and_then(module_with_providers_init)
                    else {
                        continue;
                    };
                    let imported = match namespace {
                        Some(ns) => matches!(binding(ns), Binding::Namespace),
                        None => is_import(class),
                    };
                    if !imported {
                        continue;
                    }
                    (namespace, class, Some(method))
                }
            };
            asked.push(index);
            questions.push(question);
        }
        if !questions.is_empty() {
            for (index, answer) in asked.into_iter().zip(resolve_imported(&questions)) {
                resolved[index] = answer.and_then(|module| match module {
                    // As for a same-file entry: a class declared after this
                    // NgModule isn't initialized yet when its `ɵmod` is (ngtsc
                    // wraps the list in `() => [...]` then), and an import
                    // must stay. Otherwise today's name is kept.
                    NamedClass::Local(name) => {
                        let declared_before = top_level_class(program, &name)
                            .is_some_and(|module| module.span.end <= class.span.start);
                        let is_kept = matches!(binding(&name), Binding::Import { kept: true });
                        (declared_before || is_kept)
                            .then(|| read_var(oxc_str::Ident::from(allocator.alloc_str(&name))))
                    }
                    NamedClass::Module { specifier, name } => {
                        Some(OutputExpression::ReadProp(Box::new_in(
                            ReadPropExpr {
                                receiver: Box::new_in(
                                    read_var(namespace_of(&specifier)),
                                    &allocator,
                                ),
                                name: oxc_str::Ident::from(allocator.alloc_str(&name)),
                                optional: false,
                                source_span: None,
                            },
                            &allocator,
                        )))
                    }
                });
            }
        }
    }

    if resolved.iter().all(Option::is_none) {
        return None;
    }
    Some(oxc_allocator::Vec::from_iter_in(
        resolved
            .into_iter()
            .zip(current)
            .map(|(resolved, name)| resolved.unwrap_or_else(|| read_var(*name))),
        &allocator,
    ))
}

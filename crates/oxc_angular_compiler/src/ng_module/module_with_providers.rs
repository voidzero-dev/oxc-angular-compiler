//! The `declarations`, `imports` and `exports` of a partial
//! `ɵɵngDeclareNgModule`, read like ngtsc's `resolveTypeList`.
//!
//! ngtsc evaluates each list: nested arrays, spreads and `const` lists are
//! flattened, and an entry that gives a `ModuleWithProviders`
//! (`Dep.forRoot()`, or a constant holding one) is listed as the NgModule it
//! names, not as written: `imports: [mwp, Holder.forRoot()]` becomes
//! `imports: [DepModule, DepModule]`. See
//! `NgModuleDecoratorHandler.resolveTypeList` and
//! `createModuleWithProvidersResolver` in
//! `packages/compiler-cli/src/ngtsc/annotations/ng_module/src/`.
//!
//! Oxc used to list the class a method is called on (`Holder`), which is
//! not an NgModule at all when another class holds the method, and to drop
//! spreads and namespace members. When a JIT linker inlines that list into
//! `ɵɵdefineNgModule`, the runtime gets a class that is no module and a
//! `ModuleWithProviders` object.
//!
//! The partial declaration lists what is read, and so does the full-mode
//! `ɵɵsetNgModuleScope` for a list whose entries were all read (ngtsc
//! lists the classes there too). A full-mode list with an entry Oxc can't
//! read stays as written: the runtime flattens it. The injector keeps the
//! raw entries (it needs their providers). In both modes, the lists are
//! wrapped in `() => [...]` like ngtsc does: when a name they list is
//! written after the NgModule (a class this file declares later, or an
//! import listed in a variable written later).
//!
//! A top-level entry that can't be read keeps its old output. This file
//! reads what one file can tell. With cross-file analysis, a name of
//! another file is read by `CrossFileAnalyzer::read_imported_ref`.

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
pub fn evaluated<'b, 'a>(mut expr: &'b Expression<'a>) -> &'b Expression<'a> {
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
    top_level_init(program, name, false).map(|(init, _)| init)
}

/// The initializer of the top-level `const`, `let` or `var` `name` of
/// `program`, and where its declaration starts. ngtsc's evaluator reads
/// the initializer of any variable (`getVariableValue`).
fn top_level_var_init<'b, 'a>(
    program: &'b Program<'a>,
    name: &str,
) -> Option<(&'b Expression<'a>, u32)> {
    top_level_init(program, name, true)
}

fn top_level_init<'b, 'a>(
    program: &'b Program<'a>,
    name: &str,
    any_kind: bool,
) -> Option<(&'b Expression<'a>, u32)> {
    use oxc_ast::ast::VariableDeclarationKind;
    program.body.iter().find_map(|stmt| {
        let declaration = match stmt {
            Statement::ExportDeclaration(decl) => &decl.declaration,
            _ => stmt.as_declaration()?,
        };
        let Declaration::VariableDeclaration(vars) = declaration else { return None };
        let read = match vars.kind {
            VariableDeclarationKind::Const => true,
            VariableDeclarationKind::Let | VariableDeclarationKind::Var => any_kind,
            _ => false,
        };
        if !read {
            return None;
        }
        let var = vars
            .declarations
            .iter()
            .find(|var| var.id.get_binding_identifier().is_some_and(|id| id.name == name))?;
        Some((var.init.as_ref()?, var.span.start))
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
    /// `forwardRef` of `@angular/core` imported under another name
    /// (`import { forwardRef as fr }`).
    ForwardRef,
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

/// A question to cross-file analysis about a binding the compiled file
/// imports: what `name` (or `namespace.name`) is, or what
/// `name.method(...)` gives.
#[cfg_attr(not(feature = "cross_file_elision"), expect(dead_code))]
pub struct Question<'s> {
    pub namespace: Option<&'s str>,
    pub name: &'s str,
    pub method: Option<&'s str>,
    /// Written `...name`: only a `const` array counts.
    pub spread: bool,
}

/// What cross-file analysis read for a [`Question`]. `unready` is set when
/// a class may not be defined yet as the NgModule is: its file, or the one
/// it is imported through, loads this one back.
#[cfg_attr(not(feature = "cross_file_elision"), expect(dead_code))]
pub enum Answer {
    /// A class the file imports by this name: it is listed as written.
    AsWritten { unready: bool },
    /// The classes it gives, a `const` list flattened and each
    /// `ModuleWithProviders` as its NgModule. `forward_ref` is set when
    /// the list writes a `forwardRef`.
    Classes { classes: Vec<NamedClass>, unready: bool, forward_ref: bool },
}

/// Cross-file analysis of a [`Question`]; `None` when it can't tell.
pub type ResolveImported<'r> = &'r dyn Fn(&Question<'_>) -> Option<Answer>;

/// A class a list entry names.
enum Ref {
    /// By a name of the compiled file.
    Local(String),
    /// The export `name` of `specifier`, through a namespace import.
    Module { specifier: String, name: String },
}

/// A top-level entry of a list.
struct Entry {
    /// The name the decorator parser lists for it (`extract_reference_array`).
    fallback: Option<String>,
    /// The classes it gives, `None` when unsure.
    refs: Option<Vec<Ref>>,
}

/// A list of an `@NgModule` the partial declaration names.
#[derive(Clone, Copy)]
pub enum ListKey {
    Declarations,
    Imports,
    Exports,
}

/// The `declarations`, `imports` and `exports` of an `@NgModule`, read like
/// ngtsc's `resolveTypeList`: nested arrays and `const` lists flattened,
/// each `ModuleWithProviders` listed as its NgModule, duplicates kept.
pub struct ModuleLists {
    lists: [Vec<Entry>; 3],
    /// Whether the lists must be read lazily (`() => [...]`): one names a
    /// class of this file declared after the NgModule, or lists an import
    /// by a name written after it (ngtsc's `isExpressionForwardReference`),
    /// or a `forwardRef` target that may not be initialized yet.
    pub wrap: bool,
}

/// How deep `const` lists and nested arrays are followed.
const MAX_DEPTH: u8 = 8;

/// Read the lists of the `@NgModule` `class`. `current` is what the
/// decorator parser read for `bootstrap`, `declarations`, `imports` and
/// `exports`: `None` when the entries don't line up with it. `bootstrap`
/// is only read for `wrap`. `forward_refs` is whether the parser saw a
/// `forwardRef`: it wrapped every list then, so the wrap stays unless every
/// entry is known to be defined.
pub fn read_module_lists(
    program: &Program<'_>,
    class: &Class<'_>,
    consts: &StringConsts<'_>,
    current: [&[oxc_str::Ident<'_>]; 4],
    forward_refs: bool,
    binding: impl Fn(&str) -> Binding,
    resolve_imported: Option<ResolveImported<'_>>,
) -> Option<ModuleLists> {
    let decorator = find_ng_module_decorator(&class.decorators, Some(consts))?;
    let Expression::CallExpression(call) = &decorator.expression else { return None };
    let Some(oxc_ast::ast::Argument::ObjectExpression(config)) = call.arguments.first() else {
        return None;
    };
    // The parser keeps the last key.
    let field = |name: &str| {
        config.properties.iter().rev().find_map(|property| {
            let ObjectPropertyKind::ObjectProperty(property) = property else { return None };
            let key = match &property.key {
                PropertyKey::StaticIdentifier(id) => id.name.as_str(),
                PropertyKey::StringLiteral(lit) => lit.value.as_str()?,
                _ => return None,
            };
            (crate::util::is_metadata_property(property) && key == name).then_some(&property.value)
        })
    };
    let mut walker = Walker {
        program,
        module: class,
        binding,
        resolve_imported,
        wrap: false,
        unsure: false,
        late: false,
    };
    let mut lists = ["bootstrap", "declarations", "imports", "exports"]
        .into_iter()
        .zip(current)
        .map(|(key, current)| {
            let entries = field(key).map(|value| walker.entries(value)).unwrap_or_default();
            let fallbacks = entries.iter().filter_map(|entry| entry.fallback.as_deref());
            fallbacks.eq(current.iter().map(oxc_str::Ident::as_str)).then_some(entries)
        })
        .collect::<Option<std::vec::Vec<_>>>()?
        .into_iter();
    lists.next(); // bootstrap
    let mut next = || lists.next().unwrap_or_default();
    let wrap = walker.wrap || (forward_refs && walker.unsure);
    Some(ModuleLists { lists: [next(), next(), next()], wrap })
}

impl ModuleLists {
    /// The partial declaration's `key` list, `None` to list it as before.
    /// `namespace_of` gives the namespace import of a module specifier; it
    /// is called in the order the classes are listed.
    pub fn emit<'a>(
        &self,
        allocator: &'a oxc_allocator::Allocator,
        key: ListKey,
        mut namespace_of: impl FnMut(&str) -> oxc_str::Ident<'a>,
    ) -> Option<oxc_allocator::Vec<'a, crate::output::ast::OutputExpression<'a>>> {
        use crate::output::ast::{OutputAstBuilder, OutputExpression, ReadVarExpr};
        use oxc_allocator::Box;

        let entries = &self.lists[key as usize];
        if entries.iter().all(|entry| entry.refs.is_none()) {
            return None;
        }
        let name = |name: &str| oxc_str::Ident::from(allocator.alloc_str(name));
        let read_var = |name: oxc_str::Ident<'a>| {
            OutputExpression::ReadVar(Box::new_in(
                ReadVarExpr { name, source_span: None },
                &allocator,
            ))
        };
        let mut list = oxc_allocator::Vec::new_in(&allocator);
        for entry in entries {
            let Some(refs) = &entry.refs else {
                list.extend(entry.fallback.as_deref().map(|fallback| read_var(name(fallback))));
                continue;
            };
            for class in refs {
                list.push(match class {
                    Ref::Local(local) => read_var(name(local)),
                    // `i1.X`, or `i1["h-dir"]` for an export named by a string.
                    Ref::Module { specifier, name: export } => OutputAstBuilder::export_read(
                        allocator,
                        read_var(namespace_of(specifier)),
                        name(export),
                    ),
                });
            }
        }
        Some(list)
    }

    /// Whether every entry of the `key` list was read.
    pub fn is_complete(&self, key: ListKey) -> bool {
        self.lists[key as usize].iter().all(|entry| entry.refs.is_some())
    }

    /// The `.d.ts` type reference of each class of the `key` list, read
    /// like `emit`. `type_ref(None, name)` maps a name of this file, and
    /// `type_ref(Some(specifier), name)` an export of a module. An unread
    /// entry maps its parser name. Called in the order the classes are
    /// listed.
    pub fn type_refs(
        &self,
        key: ListKey,
        mut type_ref: impl FnMut(Option<&str>, &str) -> String,
    ) -> std::vec::Vec<String> {
        let mut list = std::vec::Vec::new();
        for entry in &self.lists[key as usize] {
            let Some(refs) = &entry.refs else {
                list.extend(entry.fallback.as_deref().map(|name| type_ref(None, name)));
                continue;
            };
            list.extend(refs.iter().map(|class| match class {
                Ref::Local(name) => type_ref(None, name),
                Ref::Module { specifier, name } => type_ref(Some(specifier), name),
            }));
        }
        list
    }
}

/// The name the decorator parser lists for a top-level entry (see
/// `extract_reference_array`).
fn fallback_name(element: &ArrayExpressionElement<'_>) -> Option<String> {
    let name = match element {
        ArrayExpressionElement::Identifier(id) => &id.name,
        ArrayExpressionElement::CallExpression(call) => match &call.callee {
            Expression::Identifier(id) if id.name == "forwardRef" => {
                let Some(oxc_ast::ast::Argument::ArrowFunctionExpression(arrow)) =
                    call.arguments.first()
                else {
                    return None;
                };
                let Some(Expression::Identifier(inner)) = arrow.get_expression() else {
                    return None;
                };
                &inner.name
            }
            Expression::StaticMemberExpression(member) => match &member.object {
                Expression::Identifier(id) => &id.name,
                _ => return None,
            },
            _ => return None,
        },
        _ => return None,
    };
    Some(name.to_string())
}

/// What `forwardRef(fn)` reads, like ngtsc's `expandForwardRef`: `fn`
/// (parentheses and `as` around it looked through) is an arrow function or
/// a function expression, whose body is an expression or a block holding
/// one `return expr;`. `None` otherwise, or with more than one argument.
fn forward_ref_target<'b, 'a>(
    call: &'b oxc_ast::ast::CallExpression<'a>,
) -> Option<&'b Expression<'a>> {
    let [argument] = call.arguments.as_slice() else { return None };
    let mut function = argument.as_expression()?;
    loop {
        function = match function {
            Expression::ParenthesizedExpression(e) => &e.expression,
            Expression::TSAsExpression(e) => &e.expression,
            _ => break,
        };
    }
    let body = match function {
        Expression::ArrowFunctionExpression(arrow) => match arrow.get_expression() {
            Some(expr) => return Some(expr),
            None => arrow.get_function_body()?,
        },
        Expression::FunctionExpression(function) => function.body.as_ref()?,
        _ => return None,
    };
    // A directive is one more statement to TypeScript.
    if !body.directives.is_empty() {
        return None;
    }
    let [Statement::ReturnStatement(ret)] = body.statements.as_slice() else { return None };
    ret.argument.as_ref()
}

/// `(object, name)` of `object.name`, or of `object['name']`, which
/// ngtsc's evaluator reads the same way.
fn member_access<'b, 'a>(expr: &'b Expression<'a>) -> Option<(&'b Expression<'a>, &'b str)> {
    match expr {
        Expression::StaticMemberExpression(member) => {
            Some((&member.object, member.property.name.as_str()))
        }
        Expression::ComputedMemberExpression(member) => match evaluated(&member.expression) {
            Expression::StringLiteral(key) => Some((&member.object, key.value.as_str()?)),
            Expression::TemplateLiteral(key) if key.expressions.is_empty() => {
                Some((&member.object, key.quasis.first()?.value.cooked.as_ref()?.as_str()?))
            }
            _ => None,
        },
        _ => None,
    }
}

/// Reads list entries in the compiled file.
struct Walker<'p, 'a, 'r, B> {
    program: &'p Program<'a>,
    module: &'p Class<'a>,
    binding: B,
    resolve_imported: Option<ResolveImported<'r>>,
    wrap: bool,
    /// Set when an entry is unread, or may not be defined yet. The
    /// decorator parser wraps every list once one writes a `forwardRef`,
    /// so the wrap stays then.
    unsure: bool,
    /// Set while reading the initializer of a variable (or the method of a
    /// class) written after the NgModule. A name ngtsc writes as this file
    /// writes it there comes after the NgModule, so ngtsc wraps the lists
    /// (`isExpressionForwardReference` looks at where the written name
    /// sits).
    late: bool,
}

impl<B: Fn(&str) -> Binding> Walker<'_, '_, '_, B> {
    /// The top-level entries of a list. A value that is not an array
    /// literal (`imports: MODS`) is one entry, which lists nothing today.
    fn entries(&mut self, value: &Expression<'_>) -> std::vec::Vec<Entry> {
        let Expression::ArrayExpression(array) = value else {
            let refs = self.spread(value, 0);
            self.unsure |= refs.is_none();
            return vec![Entry { fallback: None, refs }];
        };
        array
            .elements
            .iter()
            .map(|element| {
                let refs = self.element(element, 0);
                self.unsure |= refs.is_none();
                Entry { fallback: fallback_name(element), refs }
            })
            .collect()
    }

    fn element(
        &mut self,
        element: &ArrayExpressionElement<'_>,
        depth: u8,
    ) -> Option<std::vec::Vec<Ref>> {
        match element {
            ArrayExpressionElement::SpreadElement(spread) => self.spread(&spread.argument, depth),
            ArrayExpressionElement::Elision(_) => None,
            _ => self.value(element.to_expression(), depth),
        }
    }

    /// Every element of `array` must resolve. All are read, so a later
    /// class is seen in each.
    fn array(
        &mut self,
        array: &oxc_ast::ast::ArrayExpression<'_>,
        depth: u8,
    ) -> Option<std::vec::Vec<Ref>> {
        let mut refs = Some(std::vec::Vec::new());
        for element in &array.elements {
            match (self.element(element, depth + 1), &mut refs) {
                (Some(read), Some(refs)) => refs.extend(read),
                _ => refs = None,
            }
        }
        refs
    }

    /// `...expr`: an array literal or a `const` array.
    fn spread(&mut self, expr: &Expression<'_>, depth: u8) -> Option<std::vec::Vec<Ref>> {
        if depth > MAX_DEPTH {
            return None;
        }
        match evaluated(expr) {
            Expression::ArrayExpression(array) => self.array(array, depth),
            Expression::Identifier(id) => match top_level_var_init(self.program, &id.name) {
                Some((init, start)) => self.read_at(start, |this| match evaluated(init) {
                    Expression::ArrayExpression(array) => this.array(array, depth + 1),
                    _ => None,
                }),
                None => self.ask_import(&id.name, true),
            },
            expr => {
                let (object, name) = member_access(expr)?;
                let Expression::Identifier(ns) = evaluated(object) else { return None };
                self.ask_namespace(&ns.name, name, None, true)
            }
        }
    }

    fn value(&mut self, expr: &Expression<'_>, depth: u8) -> Option<std::vec::Vec<Ref>> {
        if depth > MAX_DEPTH {
            return None;
        }
        match evaluated(expr) {
            Expression::ArrayExpression(array) => self.array(array, depth),
            Expression::Identifier(id) => self.name(&id.name, depth),
            Expression::CallExpression(call) => match &call.callee {
                Expression::Identifier(id)
                    if id.name == "forwardRef"
                        || matches!((self.binding)(&id.name), Binding::ForwardRef) =>
                {
                    self.forward_ref(call, depth)
                }
                _ => {
                    let (namespace, class, method) = static_call(call)?;
                    self.mwp_call(namespace, class, method)
                }
            },
            Expression::ObjectExpression(object) => match object_ng_module(object)? {
                ObjectModule::Value(Expression::Identifier(id)) => self.class_name(&id.name),
                _ => None,
            },
            expr => {
                let (object, name) = member_access(expr)?;
                let Expression::Identifier(ns) = evaluated(object) else { return None };
                self.ask_namespace(&ns.name, name, None, false)
            }
        }
    }

    /// A name: a class, a `const` (a list, an alias, a
    /// `ModuleWithProviders`), or an import.
    fn name(&mut self, name: &str, depth: u8) -> Option<std::vec::Vec<Ref>> {
        if depth > MAX_DEPTH {
            return None;
        }
        if self.local_class(name) {
            return Some(vec![Ref::Local(name.into())]);
        }
        let Some((init, start)) = top_level_var_init(self.program, name) else {
            return self.ask_import(name, false);
        };
        self.read_at(start, |this| match evaluated(init) {
            Expression::ArrayExpression(array) => this.array(array, depth + 1),
            Expression::Identifier(alias) => this.name(&alias.name, depth + 1),
            init => match module_with_providers_init(init)? {
                MwpInit::Call { namespace, class, method } => {
                    this.mwp_call(namespace, class, method)
                }
                MwpInit::Module(Expression::Identifier(id)) => this.class_name(&id.name),
                MwpInit::Module(_) => None,
            },
        })
    }

    /// Run `read` with [`Self::late`] set by whether `start`, where what it
    /// reads is written, comes after the NgModule.
    fn read_at<T>(&mut self, start: u32, read: impl FnOnce(&mut Self) -> T) -> T {
        let late = std::mem::replace(&mut self.late, start > self.module.span.start);
        let result = read(self);
        self.late = late;
        result
    }

    /// `class.method(...)` or `namespace.class.method(...)`.
    fn mwp_call(
        &mut self,
        namespace: Option<&str>,
        class: &str,
        method: &str,
    ) -> Option<std::vec::Vec<Ref>> {
        let program = self.program;
        if let Some(namespace) = namespace {
            return self.ask_namespace(namespace, class, Some(method), false);
        }
        if let Some(declared) = top_level_class(program, class) {
            // The NgModule is named in the method, where the class is.
            let module = method_module(program, class, method)?;
            return self.read_at(declared.span.start, |this| this.class_name(module));
        }
        if !matches!((self.binding)(class), Binding::Import { .. }) {
            return None;
        }
        let question =
            Question { namespace: None, name: class, method: Some(method), spread: false };
        let answer = self.ask(&question)?;
        self.classes(answer)
    }

    /// `forwardRef(fn)`: what `fn` returns, read like any entry (ngtsc's
    /// `createForwardRefResolver`). ngtsc wraps the lists only when a name
    /// it writes sits after the NgModule (see `Walker::local_class` and
    /// `Walker::late`). Oxc also keeps the wrap
    /// when the target is unread, or may not be defined yet as the NgModule
    /// is (its file loads this one back): the full-mode scope runs with no
    /// `ngJitMode` guard, so reading it early would throw.
    fn forward_ref(
        &mut self,
        call: &oxc_ast::ast::CallExpression<'_>,
        depth: u8,
    ) -> Option<std::vec::Vec<Ref>> {
        let Some(target) = forward_ref_target(call) else {
            self.wrap = true;
            return None;
        };
        let unsure = std::mem::replace(&mut self.unsure, false);
        let refs = self.value(target, depth + 1);
        self.wrap |= refs.is_none() || self.unsure;
        self.unsure |= unsure;
        refs
    }

    /// A name this file imports.
    fn ask_import(&mut self, name: &str, spread: bool) -> Option<std::vec::Vec<Ref>> {
        let Binding::Import { kept } = (self.binding)(name) else { return None };
        let question = Question { namespace: None, name, method: None, spread };
        match self.ask(&question)? {
            Answer::AsWritten { unready } => {
                self.unsure |= unready;
                let listed = kept && !spread;
                // Listed by the name written here; see `Walker::late`.
                self.wrap |= listed && self.late;
                listed.then(|| vec![Ref::Local(name.into())])
            }
            answer @ Answer::Classes { .. } => self.classes(answer),
        }
    }

    /// `namespace.name`, or `namespace.name.method(...)`.
    fn ask_namespace(
        &mut self,
        namespace: &str,
        name: &str,
        method: Option<&str>,
        spread: bool,
    ) -> Option<std::vec::Vec<Ref>> {
        if !matches!((self.binding)(namespace), Binding::Namespace) {
            return None;
        }
        let question = Question { namespace: Some(namespace), name, method, spread };
        let answer = self.ask(&question)?;
        self.classes(answer)
    }

    fn ask(&self, question: &Question<'_>) -> Option<Answer> {
        (self.resolve_imported?)(question)
    }

    fn classes(&mut self, answer: Answer) -> Option<std::vec::Vec<Ref>> {
        let Answer::Classes { classes, unready, forward_ref } = answer else { return None };
        // A `forwardRef` in the list asks for the wrap; see `Walker::unsure`
        // for the others.
        self.wrap |= forward_ref && unready;
        self.unsure |= unready;
        classes
            .into_iter()
            .map(|class| match class {
                NamedClass::Local(name) => self.local_class(&name).then_some(Ref::Local(name)),
                NamedClass::Module { specifier, name } => Some(Ref::Module { specifier, name }),
            })
            .collect()
    }

    /// A class this file declares or a kept import, listed by `name`.
    fn class_name(&mut self, name: &str) -> Option<std::vec::Vec<Ref>> {
        if self.local_class(name) {
            return Some(vec![Ref::Local(name.into())]);
        }
        let listed = matches!((self.binding)(name), Binding::Import { kept: true });
        // Listed by the name written here; see `Walker::late`.
        self.wrap |= listed && self.late;
        listed.then(|| vec![Ref::Local(name.into())])
    }

    /// Whether `name` is a top-level class of this file. One declared after
    /// the NgModule isn't initialized when its definition is: the lists
    /// are wrapped then, like ngtsc does.
    fn local_class(&mut self, name: &str) -> bool {
        let Some(class) = top_level_class(self.program, name) else { return false };
        if class.span.start > self.module.span.start {
            self.wrap = true;
        }
        true
    }
}

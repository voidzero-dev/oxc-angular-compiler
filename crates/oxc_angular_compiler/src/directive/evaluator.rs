//! A single-file model of ngtsc's partial evaluator.
//!
//! Decorator metadata such as `inputs:`, `outputs:` and `queries:` is read by
//! ngtsc through its partial evaluator
//! (packages/compiler-cli/src/ngtsc/partial_evaluator), and its diagnostics
//! describe values the way that evaluator resolves them. This module reproduces
//! the parts of it that can be answered from the current file, so the compiled
//! metadata and the diagnostics match ngtsc word for word. Imported bindings
//! can't be seen into and are treated as opaque references.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    ArrayExpressionElement, ArrowFunctionExpression, BindingPattern, Class, ClassElement,
    Declaration, ExportDefaultDeclarationKind, Expression, FormalParameters, Function,
    ImportDeclarationSpecifier, MethodDefinitionKind, ModuleExportName, ObjectPropertyKind,
    Program, PropertyKey, Statement, TSType, TSTypeName,
};
use oxc_ast_visit::Visit;

use crate::output::emitter::format_number_like_js;

/// Everything declared at the top level of a file that the evaluator can resolve.
#[derive(Default)]
pub(crate) struct FileScope<'a> {
    /// `const`/`let`/`var` bindings with an initializer.
    variables: HashMap<&'a str, &'a Expression<'a>>,
    /// Bindings with no initializer (`declare const X: T`, `let x;`) and enums.
    declared: HashSet<&'a str>,
    /// Function declarations, with their overload signature count.
    functions: HashMap<&'a str, (&'a Function<'a>, usize)>,
    classes: HashMap<&'a str, &'a Class<'a>>,
    imports: HashMap<&'a str, Import<'a>>,
    /// Names exported from the file (`export ...` and `export { ... }`).
    exported: HashSet<&'a str>,
    /// Interfaces, type aliases, classes and enums declared in the file.
    types: HashSet<&'a str>,
}

/// An import binding: the module it comes from and, unless it's a namespace
/// import, the name it's exported under.
#[derive(Clone, Copy)]
pub(crate) struct Import<'a> {
    pub module: &'a str,
    pub imported: Option<&'a str>,
}

impl<'a> FileScope<'a> {
    pub(crate) fn collect(program: &'a Program<'a>) -> Self {
        let mut scope = Self::default();
        for stmt in &program.body {
            match stmt {
                Statement::ImportDeclaration(import) => {
                    let module = import.source.value.as_str();
                    for spec in import.specifiers.iter().flatten() {
                        let (local, imported) = match spec {
                            ImportDeclarationSpecifier::ImportSpecifier(s) => {
                                let imported = match &s.imported {
                                    ModuleExportName::IdentifierName(n) => n.name.as_str(),
                                    ModuleExportName::IdentifierReference(n) => n.name.as_str(),
                                    ModuleExportName::StringLiteral(n) => n.value.as_str(),
                                };
                                (s.local.name.as_str(), Some(imported))
                            }
                            ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                                (s.local.name.as_str(), Some("default"))
                            }
                            ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                                (s.local.name.as_str(), None)
                            }
                        };
                        scope.imports.insert(local, Import { module, imported });
                    }
                }
                Statement::ExportDeclaration(export) => {
                    scope.declaration(&export.declaration, true);
                }
                Statement::ExportNamedDeclaration(export) => {
                    for spec in &export.specifiers {
                        if let ModuleExportName::IdentifierReference(local) = &spec.local {
                            scope.exported.insert(local.name.as_str());
                        }
                    }
                }
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    ExportDefaultDeclarationKind::ClassDeclaration(class) => {
                        scope.class(class, true);
                    }
                    ExportDefaultDeclarationKind::FunctionDeclaration(function) => {
                        scope.function(function, true);
                    }
                    _ => {}
                },
                _ => {
                    if let Some(decl) = stmt.as_declaration() {
                        scope.declaration(decl, false);
                    }
                }
            }
        }
        scope
    }

    fn declaration(&mut self, decl: &'a Declaration<'a>, exported: bool) {
        let name = |scope: &mut Self, name: &'a str| {
            if exported {
                scope.exported.insert(name);
            }
        };
        match decl {
            Declaration::VariableDeclaration(vars) => {
                for var in &vars.declarations {
                    let BindingPattern::BindingIdentifier(id) = &var.id else { continue };
                    let id = id.name.as_str();
                    match &var.init {
                        Some(init) => {
                            self.variables.insert(id, init);
                        }
                        None => {
                            self.declared.insert(id);
                        }
                    }
                    name(self, id);
                }
            }
            Declaration::FunctionDeclaration(function) => self.function(function, exported),
            Declaration::ClassDeclaration(class) => self.class(class, exported),
            Declaration::TSEnumDeclaration(e) => {
                let id = e.id.name.as_str();
                // An enum isn't evaluated; it's a non-function reference.
                self.declared.insert(id);
                self.types.insert(id);
                name(self, id);
            }
            Declaration::TSInterfaceDeclaration(i) => {
                self.types.insert(i.id.name.as_str());
                name(self, i.id.name.as_str());
            }
            Declaration::TSTypeAliasDeclaration(t) => {
                self.types.insert(t.id.name.as_str());
                name(self, t.id.name.as_str());
            }
            _ => {}
        }
    }

    fn function(&mut self, function: &'a Function<'a>, exported: bool) {
        let Some(id) = &function.id else { return };
        let id = id.name.as_str();
        if exported {
            self.exported.insert(id);
        }
        // Overloads are body-less declarations followed by the implementation;
        // only the overloads count as call signatures, as in TypeScript.
        let entry = self.functions.entry(id).or_insert((function, 0));
        if function.body.is_none() {
            entry.1 += 1;
        }
        if function.body.is_some() || entry.0.body.is_none() {
            entry.0 = function;
        }
    }

    fn class(&mut self, class: &'a Class<'a>, exported: bool) {
        let Some(id) = &class.id else { return };
        let id = id.name.as_str();
        self.classes.insert(id, class);
        self.types.insert(id);
        if exported {
            self.exported.insert(id);
        }
    }

    pub(crate) fn import(&self, name: &str) -> Option<Import<'a>> {
        self.imports.get(name).copied()
    }
}

/// A function definition ngtsc's `getDefinitionOfFunction` accepts.
#[derive(Clone, Copy)]
pub(crate) enum FnDef<'a> {
    Function(&'a Function<'a>),
    Arrow(&'a ArrowFunctionExpression<'a>),
}

impl<'a> FnDef<'a> {
    fn is_generic(self) -> bool {
        match self {
            FnDef::Function(f) => f.type_parameters.is_some(),
            FnDef::Arrow(f) => f.type_parameters.is_some(),
        }
    }

    pub(crate) fn params(self) -> &'a FormalParameters<'a> {
        match self {
            FnDef::Function(f) => &f.params,
            FnDef::Arrow(f) => &f.params,
        }
    }

    /// The type annotation of the first parameter (after any `this` parameter,
    /// which oxc keeps separately): `Ok(None)` when there are no parameters,
    /// `Err(())` when the first one has no type.
    pub(crate) fn first_param_type(self) -> Result<Option<&'a TSType<'a>>, ()> {
        let params = self.params();
        let annotation = match (params.items.first(), &params.rest) {
            (Some(param), _) => &param.type_annotation,
            (None, Some(rest)) => &rest.type_annotation,
            (None, None) => return Ok(None),
        };
        annotation.as_ref().map(|t| Some(&t.type_annotation)).ok_or(())
    }
}

/// What a declaration reference resolves to.
#[derive(Clone)]
pub(crate) enum RefKind<'a> {
    /// A function declaration or static method in this file, with its call
    /// signature count and whether it's a top-level function declaration
    /// (whose own name is in scope where the metadata is compiled).
    Function(FnDef<'a>, usize, bool),
    Class(&'a Class<'a>),
    /// An imported binding; `namespace_member` for `ns.x` through `import * as ns`.
    Import {
        namespace_member: bool,
    },
    /// An identifier with no declaration in this file (a global, most likely).
    Global,
    /// Any other declaration (a variable without an initializer, ...).
    Other,
}

/// A value as ngtsc's partial evaluator resolves it.
#[derive(Clone)]
pub(crate) enum Value<'a> {
    Null,
    Undefined,
    Bool(bool),
    Number(f64),
    String(String),
    Array(Vec<Value<'a>>),
    Object(Vec<Prop<'a>>),
    /// `import * as ns`.
    Module,
    Reference {
        name: String,
        kind: RefKind<'a>,
    },
    Dynamic,
    /// An arrow or function expression written directly as a property value;
    /// the only place ngtsc keeps a function expression analyzable.
    Function(FnDef<'a>),
}

/// An object literal property: its key, value, and the source expression it came from.
#[derive(Clone)]
pub(crate) struct Prop<'a> {
    pub key: String,
    pub value: Value<'a>,
    pub expr: Option<&'a Expression<'a>>,
}

impl<'a> Value<'a> {
    pub(crate) fn prop(&self, key: &str) -> Option<&Prop<'a>> {
        match self {
            Value::Object(props) => props.iter().rev().find(|p| p.key == key),
            _ => None,
        }
    }

    pub(crate) fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// ngtsc's `describeResolvedType`, one level deep like its diagnostics.
    pub(crate) fn describe(&self) -> String {
        self.describe_to(1)
    }

    fn describe_to(&self, depth: u16) -> String {
        match self {
            Value::Null => "null".into(),
            Value::Undefined => "undefined".into(),
            Value::Bool(_) => "boolean".into(),
            Value::Number(_) => "number".into(),
            Value::String(_) => "string".into(),
            Value::Object(_) if depth == 0 => "object".into(),
            Value::Object(props) if props.is_empty() => "{}".into(),
            Value::Object(props) => {
                let mut seen = HashSet::new();
                // Later duplicates win but keep the first key's position, like a Map.
                let entries: std::vec::Vec<String> = props
                    .iter()
                    .filter(|p| seen.insert(p.key.as_str()))
                    .map(|p| {
                        let value = self.prop(&p.key).map_or(&p.value, |p| &p.value);
                        format!("{}: {}", quote_key(&p.key), value.describe_to(depth - 1))
                    })
                    .collect();
                format!("{{ {} }}", entries.join("; "))
            }
            Value::Array(_) if depth == 0 => "Array".into(),
            Value::Array(items) => format!(
                "[{}]",
                items
                    .iter()
                    .map(|v| v.describe_to(depth - 1))
                    .collect::<std::vec::Vec<_>>()
                    .join(", ")
            ),
            Value::Module => "(module)".into(),
            Value::Reference { name, .. } => name.clone(),
            Value::Dynamic | Value::Function(_) => "(not statically analyzable)".into(),
        }
    }

    /// The chained line ngtsc's `createValueHasWrongTypeError` adds after a message.
    pub(crate) fn wrong_type_suffix(&self) -> String {
        match self {
            Value::Dynamic | Value::Function(_) => {
                " Value could not be determined statically.".into()
            }
            Value::Reference { name, .. } => format!(" Value is a reference to '{name}'."),
            _ => format!(" Value is of type '{}'.", self.describe()),
        }
    }
}

fn quote_key(key: &str) -> String {
    if !key.is_empty() && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        key.to_string()
    } else {
        format!("'{}'", key.replace('\'', "\\'"))
    }
}

/// Bounds expression nesting, to keep deeply nested literals off the end of the stack.
const MAX_DEPTH: u16 = 256;

pub(crate) struct Evaluator<'s, 'a> {
    /// The file's declarations are only looked at when an identifier is resolved.
    consts: &'s super::StringConsts<'a>,
    /// Top-level variables already evaluated, so a chain of consts that each
    /// reference the previous one several times stays linear. `None` while a
    /// variable is being evaluated, which makes a circular reference dynamic.
    variables: std::cell::RefCell<HashMap<&'a str, Option<Value<'a>>>>,
}

impl<'s, 'a> Evaluator<'s, 'a> {
    pub(crate) fn new(consts: &'s super::StringConsts<'a>) -> Self {
        Self { consts, variables: std::cell::RefCell::default() }
    }

    pub(crate) fn evaluate(&self, expr: &'a Expression<'a>) -> Value<'a> {
        self.eval(expr, 0)
    }

    fn eval(&self, expr: &'a Expression<'a>, depth: u16) -> Value<'a> {
        if depth > MAX_DEPTH {
            return Value::Dynamic;
        }
        let depth = depth + 1;
        match expr {
            Expression::NullLiteral(_) => Value::Null,
            Expression::BooleanLiteral(b) => Value::Bool(b.value),
            Expression::NumericLiteral(n) => Value::Number(n.value),
            Expression::StringLiteral(s) => Value::String(s.value.to_string()),
            Expression::TemplateLiteral(tpl) => {
                let mut out = String::new();
                for (i, quasi) in tpl.quasis.iter().enumerate() {
                    let Some(cooked) = &quasi.value.cooked else { return Value::Dynamic };
                    out.push_str(cooked);
                    if let Some(e) = tpl.expressions.get(i) {
                        match self.eval(e, depth) {
                            Value::String(s) => out.push_str(&s),
                            Value::Number(n) => out.push_str(&format_number_like_js(n)),
                            Value::Bool(b) => out.push_str(if b { "true" } else { "false" }),
                            Value::Null => out.push_str("null"),
                            Value::Undefined => out.push_str("undefined"),
                            _ => return Value::Dynamic,
                        }
                    }
                }
                Value::String(out)
            }
            Expression::Identifier(id) => self.identifier(id.name.as_str(), depth),
            Expression::ParenthesizedExpression(e) => self.eval(&e.expression, depth),
            Expression::TSAsExpression(e) => self.eval(&e.expression, depth),
            Expression::TSSatisfiesExpression(e) => self.eval(&e.expression, depth),
            Expression::TSNonNullExpression(e) => self.eval(&e.expression, depth),
            Expression::TSTypeAssertion(e) => self.eval(&e.expression, depth),
            Expression::ArrayExpression(arr) => {
                let mut items = std::vec::Vec::new();
                for el in &arr.elements {
                    match el {
                        ArrayExpressionElement::SpreadElement(spread) => {
                            match self.eval(&spread.argument, depth) {
                                Value::Array(inner) => items.extend(inner),
                                _ => return Value::Dynamic,
                            }
                        }
                        ArrayExpressionElement::Elision(_) => items.push(Value::Dynamic),
                        _ => items.push(self.eval(el.to_expression(), depth)),
                    }
                }
                Value::Array(items)
            }
            Expression::ObjectExpression(obj) => {
                let mut props = std::vec::Vec::new();
                for prop in &obj.properties {
                    match prop {
                        ObjectPropertyKind::ObjectProperty(p) => {
                            if p.method || !matches!(p.kind, oxc_ast::ast::PropertyKind::Init) {
                                return Value::Dynamic;
                            }
                            let Some(key) = self.property_key(&p.key, depth) else {
                                return Value::Dynamic;
                            };
                            let value = match &p.value {
                                Expression::ArrowFunctionExpression(f) => {
                                    Value::Function(FnDef::Arrow(f))
                                }
                                Expression::FunctionExpression(f) => {
                                    Value::Function(FnDef::Function(f))
                                }
                                value => self.eval(value, depth),
                            };
                            props.push(Prop { key, value, expr: Some(&p.value) });
                        }
                        ObjectPropertyKind::SpreadProperty(spread) => {
                            match self.eval(&spread.argument, depth) {
                                Value::Object(inner) => props.extend(inner),
                                _ => return Value::Dynamic,
                            }
                        }
                    }
                }
                Value::Object(props)
            }
            // A function expression reached through a property access isn't
            // analyzable (`({ f: (v: string) => 1 }).f`).
            Expression::StaticMemberExpression(m) => {
                let object = self.eval(&m.object, depth);
                opaque_function(self.member(object, m.property.name.as_str(), depth))
            }
            Expression::ComputedMemberExpression(m) => {
                let object = self.eval(&m.object, depth);
                opaque_function(match self.eval(&m.expression, depth) {
                    Value::String(key) => self.member(object, &key, depth),
                    Value::Number(n) => self.member(object, &format_number_like_js(n), depth),
                    _ => Value::Dynamic,
                })
            }
            _ => Value::Dynamic,
        }
    }

    fn property_key(&self, key: &'a PropertyKey<'a>, depth: u16) -> Option<String> {
        match key {
            PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
            PropertyKey::PrivateIdentifier(_) => None,
            key => match self.eval(key.to_expression(), depth) {
                Value::String(s) => Some(s),
                Value::Number(n) => Some(format_number_like_js(n)),
                _ => None,
            },
        }
    }

    fn identifier(&self, name: &'a str, depth: u16) -> Value<'a> {
        let scope = self.consts.scope();
        if let Some(init) = scope.variables.get(name) {
            if let Some(cached) = self.variables.borrow().get(name) {
                return cached.clone().unwrap_or(Value::Dynamic);
            }
            self.variables.borrow_mut().insert(name, None);
            // A dynamic value reached through a declaration loses its node, so a
            // function expression assigned to a variable isn't analyzable.
            let value = opaque_function(self.eval(init, depth));
            self.variables.borrow_mut().insert(name, Some(value.clone()));
            return value;
        }
        if let Some((f, overloads)) = scope.functions.get(name) {
            return Value::Reference {
                name: name.into(),
                kind: RefKind::Function(FnDef::Function(f), (*overloads).max(1), true),
            };
        }
        if let Some(class) = scope.classes.get(name) {
            return Value::Reference { name: name.into(), kind: RefKind::Class(class) };
        }
        if let Some(import) = scope.imports.get(name) {
            return match import.imported {
                Some(imported) => {
                    let name = if imported == "default" { name } else { imported };
                    Value::Reference {
                        name: name.into(),
                        kind: RefKind::Import { namespace_member: false },
                    }
                }
                None => Value::Module,
            };
        }
        if scope.declared.contains(name) {
            return Value::Reference { name: name.into(), kind: RefKind::Other };
        }
        match name {
            "undefined" => Value::Undefined,
            _ => Value::Reference { name: name.into(), kind: RefKind::Global },
        }
    }

    fn member(&self, object: Value<'a>, key: &str, depth: u16) -> Value<'a> {
        match object {
            Value::Object(props) => {
                props.into_iter().rev().find(|p| p.key == key).map_or(Value::Undefined, |p| p.value)
            }
            Value::Array(items) => match key {
                "length" => Value::Number(items.len() as f64),
                _ => key
                    .parse::<usize>()
                    .ok()
                    .and_then(|i| items.into_iter().nth(i))
                    .unwrap_or(Value::Undefined),
            },
            Value::String(s) if key == "length" => Value::Number(s.chars().count() as f64),
            Value::Module => Value::Reference {
                name: key.into(),
                kind: RefKind::Import { namespace_member: true },
            },
            Value::Reference { kind: RefKind::Class(class), .. } => {
                self.static_member(class, key, depth)
            }
            Value::Reference { kind: RefKind::Import { .. } | RefKind::Global, .. } => {
                Value::Reference { name: key.into(), kind: RefKind::Global }
            }
            _ => Value::Dynamic,
        }
    }

    fn static_member(&self, class: &'a Class<'a>, key: &str, depth: u16) -> Value<'a> {
        let mut overloads = 0;
        let mut found = None;
        for element in &class.body.body {
            match element {
                ClassElement::MethodDefinition(m)
                    if m.r#static
                        && m.kind == MethodDefinitionKind::Method
                        && m.key.static_name().is_some_and(|n| n == key) =>
                {
                    if m.value.body.is_none() {
                        overloads += 1;
                    } else {
                        found = Some(FnDef::Function(&m.value));
                    }
                }
                ClassElement::PropertyDefinition(p)
                    if p.r#static && p.key.static_name().is_some_and(|n| n == key) =>
                {
                    return p.value.as_ref().map_or(Value::Undefined, |v| {
                        match self.eval(v, depth) {
                            Value::Function(_) => Value::Dynamic,
                            value => value,
                        }
                    });
                }
                _ => {}
            }
        }
        match found {
            Some(def) => Value::Reference {
                name: key.into(),
                kind: RefKind::Function(def, overloads.max(1), false),
            },
            _ if key == "prototype" => Value::Dynamic,
            _ => Value::Undefined,
        }
    }
}

fn opaque_function(value: Value<'_>) -> Value<'_> {
    match value {
        Value::Function(_) => Value::Dynamic,
        value => value,
    }
}

// =============================================================================
// Input transforms
// =============================================================================

/// ngtsc's checks on an input `transform`
/// (`parseDecoratorInputTransformFunction` in
/// packages/compiler-cli/src/ngtsc/annotations/directive/src/input_transforms.ts).
///
/// `position` is the index in the `inputs:` array, `None` for `@Input({ transform })`.
pub(crate) fn transform_error(
    value: &Value<'_>,
    position: Option<usize>,
    input_name: &str,
    class: &Class<'_>,
    scope: &FileScope<'_>,
) -> Option<String> {
    let suffix = value.wrong_type_suffix();
    let def = match value {
        Value::Function(def) | Value::Reference { kind: RefKind::Function(def, ..), .. } => *def,
        // Imports and globals can't be inspected from this file; assume a function.
        Value::Reference {
            kind: RefKind::Import { namespace_member: false } | RefKind::Global,
            ..
        } => {
            return None;
        }
        Value::Reference { kind: RefKind::Import { namespace_member: true }, .. } => {
            return Some(format!("Input transform function could not be referenced{suffix}"));
        }
        Value::Reference { .. } | Value::Dynamic => {
            return Some(format!("Input transform must be a function{suffix}"));
        }
        _ => {
            return Some(match position {
                Some(i) => format!(
                    "Transform of value at position {i} of @Directive.inputs array must be a function{suffix}"
                ),
                None => format!("Input transform must be a function{suffix}"),
            });
        }
    };
    if def.is_generic() {
        return Some(format!("Input transform function cannot be generic{suffix}"));
    }
    if let Value::Reference { kind: RefKind::Function(_, signatures, _), .. } = value
        && *signatures > 1
    {
        return Some(format!("Input transform function cannot have multiple signatures{suffix}"));
    }
    let conflicting = format!("ngAcceptInputType_{input_name}");
    if class.body.body.iter().any(|el| {
        el.r#static()
            && el
                .property_key()
                .and_then(PropertyKey::static_name)
                .is_some_and(|n| n == conflicting)
    }) {
        return Some(format!(
            "Class cannot have both a transform function on Input {input_name} and a static member called {conflicting}"
        ));
    }
    match def.first_param_type() {
        Ok(None) => None,
        Err(()) => {
            Some(format!("Input transform function first parameter must have a type{suffix}"))
        }
        Ok(Some(_)) if def.params().items.is_empty() => Some(format!(
            "Input transform function first parameter cannot be a spread parameter{suffix}"
        )),
        Ok(Some(ty)) => {
            let mut check = UnexportedType { scope, found: false };
            check.visit_ts_type(ty);
            check.found.then(|| {
                "Symbol must be exported in order to be used as the type of an Input transform function"
                    .to_string()
            })
        }
    }
}

/// Finds a type reference to a same-file type that isn't exported, which ngtsc
/// can't emit into the `.d.ts` (`assertEmittableInputType`).
struct UnexportedType<'s, 'a> {
    scope: &'s FileScope<'a>,
    found: bool,
}

impl<'a> Visit<'a> for UnexportedType<'_, 'a> {
    fn visit_ts_type_name(&mut self, name: &TSTypeName<'a>) {
        if let TSTypeName::IdentifierReference(id) = name {
            let id = id.name.as_str();
            if self.scope.types.contains(id) && !self.scope.exported.contains(id) {
                self.found = true;
            }
        }
        oxc_ast_visit::walk::walk_ts_type_name(self, name);
    }
}

// =============================================================================
// `.d.ts` input transform types
// =============================================================================

/// The `.d.ts` type of each input transform's first parameter, for
/// `static ngAcceptInputType_<name>: T;`.
///
/// Only transforms defined in this file are covered; imported ones can't be
/// inspected and are left out.
pub fn input_transform_types<'a>(
    class: &'a Class<'a>,
    consts: &super::StringConsts<'a>,
    source: &'a str,
) -> HashMap<String, String> {
    let evaluator = Evaluator::new(consts);
    // The transform that ends up compiled for each input: a member `@Input`
    // overrides an `inputs:` entry, like the compiled inputs map.
    let mut transforms: std::vec::Vec<(String, Option<FnDef<'a>>)> = std::vec::Vec::new();
    let mut record = |name: String, value: &Value<'a>| {
        let def = match value {
            Value::Function(def) | Value::Reference { kind: RefKind::Function(def, ..), .. } => {
                Some(*def)
            }
            _ => None,
        };
        match transforms.iter_mut().find(|(n, _)| *n == name) {
            Some(entry) => entry.1 = def,
            None => transforms.push((name, def)),
        }
    };
    if let Some((Some(config), _)) = super::angular_decorator_config(class)
        && let Some(inputs) = super::decorator::config_property(config, "inputs", consts)
        && let Value::Array(items) = evaluator.evaluate(inputs)
    {
        for item in &items {
            if let (Some(Value::String(name)), Some(transform)) =
                (item.prop("name").map(|p| &p.value), item.prop("transform"))
            {
                record(name.clone(), &transform.value);
            }
        }
    }
    for element in &class.body.body {
        let (key, decorators) = match element {
            ClassElement::PropertyDefinition(p) => (&p.key, &p.decorators),
            ClassElement::AccessorProperty(p) => (&p.key, &p.decorators),
            ClassElement::MethodDefinition(m) => (&m.key, &m.decorators),
            _ => continue,
        };
        let Some(name) = key.static_name() else { continue };
        if let Some(options) = super::property_decorators::input_decorator_options(decorators)
            && let Some(transform) = evaluator.evaluate(options).prop("transform")
        {
            record(name.to_string(), &transform.value);
        }
    }

    transforms
        .into_iter()
        .filter_map(|(name, def)| {
            let ty = match def?.first_param_type().ok()? {
                Some(ty) => {
                    let mut printer = super::dts_type::TypePrinter {
                        scope: consts.scope(),
                        source,
                        other_module: false,
                    };
                    let ty = printer.print(ty);
                    if printer.other_module {
                        return None;
                    }
                    ty
                }
                None => "unknown".to_string(),
            };
            Some((name, ty))
        })
        .collect()
}

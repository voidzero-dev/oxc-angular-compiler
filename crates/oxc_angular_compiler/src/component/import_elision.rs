//! Angular-compatible import elision using oxc's semantic reference tracking.
//!
//! This module uses oxc's `ReferenceFlags` to determine which imports have value references
//! vs type-only references. This is the same approach used by oxc's TypeScript transformer.
//!
//! ## How it works
//!
//! oxc's semantic analysis tracks every reference to a symbol with `ReferenceFlags`:
//! - `Type` flag: Symbol is used in a type position (type annotation, implements, etc.)
//! - `Read`/`Write` flags: Symbol is used as a value (expressions, decorator args, etc.)
//!
//! An import should be elided if ALL its references have ONLY the `Type` flag.
//!
//! ## What gets elided
//!
//! - `import type { X }` - explicit type-only imports
//! - `import { type X }` - explicit type-only specifiers
//! - Types only used in type annotations (e.g., `userId?: UserId`)
//! - Interfaces only used in `implements` clause
//! - Constructor parameter types (DI tokens are provided via namespace imports)
//! - Constructor parameter decorators (`@Inject`, `@Optional`, etc.) - Angular removes these
//! - DI tokens used only in `@Inject(TOKEN)` arguments
//!
//! The last two apply only when no `ɵsetClassMetadata` is emitted: the
//! metadata's `ctorParameters` callback names the param decorators and
//! `@Inject` tokens as bare identifiers, so their imports must survive
//! (matching ngtsc, which keeps them).
//!
//! ## What gets preserved
//!
//! - Decorators used at runtime (@Component, @Input, etc.)
//! - Values used in expressions (new X(), call X(), etc.)
//! - Any import with at least one value reference outside of constructor parameter context
//!
//! ## Cross-file analysis (optional)
//!
//! When the `cross_file_elision` feature is enabled, additional analysis can be performed
//! to check if imported symbols are type-only in their source files. This is useful for
//! compare tests but is not needed in production (bundlers handle this).

#[cfg(feature = "cross_file_elision")]
use std::path::Path;

use oxc_ast::ast::{
    BindingIdentifier, ClassElement, Expression, ImportDeclarationSpecifier, MethodDefinitionKind,
    Program, Statement, StringLiteral, TSType,
};
use oxc_ast_visit::Visit;
use oxc_semantic::{ReferenceId, Semantic, SemanticBuilder, SymbolFlags};
use oxc_str::Ident;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::optimizer::Edit;

/// Analyzer for determining which imports are type-only and can be elided.
pub struct ImportElisionAnalyzer<'a> {
    /// Set of import specifier local names that should be removed (type-only).
    type_only_specifiers: FxHashSet<Ident<'a>>,
    /// Number of value (non-type) references to each named or default import
    /// specifier, by local name.
    value_reference_counts: FxHashMap<Ident<'a>, usize>,
    /// Imports the compiled output references as values although the source
    /// doesn't (see [`Self::preserve`] and the post-passes in
    /// [`Self::analyze`]), so `@defer` can't remove them.
    kept_by_output: FxHashSet<Ident<'a>>,
    /// The non-`type` specifiers of each import declaration, by local name.
    declaration_specifiers: Vec<Vec<Ident<'a>>>,
    /// Index into `declaration_specifiers` of each non-`type` specifier.
    declaration_of: FxHashMap<Ident<'a>, usize>,
}

impl<'a> ImportElisionAnalyzer<'a> {
    /// Analyze a program to determine which imports are type-only.
    ///
    /// This builds a semantic model from the program and checks each import
    /// specifier to see if it has any non-type references.
    ///
    /// `emit_metadata` must mirror whether the transform will emit
    /// `ɵsetClassMetadata` for this file (`emit_class_metadata &&
    /// !advanced_optimizations`). The metadata callback names constructor
    /// parameter decorators (`{type: Optional}`), `@Inject(TOKEN)` argument
    /// tokens (`args: [TOKEN]`), and member decorators (`{type: Input}`) by
    /// their local (imported) name, so those imports must survive elision —
    /// like ngtsc, which keeps them. When no metadata is emitted, the
    /// decorators are stripped outright and the imports can be dropped.
    pub fn analyze(program: &'a Program<'a>, emit_metadata: bool) -> Self {
        let semantic_ret = SemanticBuilder::new().build(program);
        Self::analyze_with_semantic(program, &semantic_ret.semantic, emit_metadata)
    }

    /// [`Self::analyze`] with a semantic model the caller already built for
    /// `program`, so one build can serve several passes.
    pub fn analyze_with_semantic(
        program: &'a Program<'a>,
        semantic: &Semantic<'a>,
        emit_metadata: bool,
    ) -> Self {
        let mut type_only_specifiers = FxHashSet::default();
        let mut value_reference_counts = FxHashMap::default();
        let mut declaration_specifiers = Vec::new();
        let mut declaration_of = FxHashMap::default();

        // The references inside decorators compilation removes: an import is decorator-only when every value
        // reference to it is one of them. When class metadata is emitted, `ctorParameters`/`propDecorators`
        // name the same identifiers, so none is removed.
        let consts = crate::directive::StringConsts::declarations_of(program);
        let removed_references = if emit_metadata {
            FxHashSet::default()
        } else {
            Self::removed_decorator_references(program, &consts)
        };

        // Analyze each import declaration
        for stmt in &program.body {
            let Statement::ImportDeclaration(import_decl) = stmt else {
                continue;
            };

            // Skip type-only imports entirely (import type { X })
            // These are already handled by TypeScript stripping
            if import_decl.import_kind.is_type() {
                continue;
            }

            // A source with a lone surrogate (`'\uD800'`) can't be named in a
            // generated import (`build_import_map` leaves it out), so generated
            // code references its bindings as written: keep them all.
            if import_decl.source.value.has_lone_surrogate() {
                continue;
            }

            let Some(specifiers) = &import_decl.specifiers else {
                continue;
            };

            let declaration = declaration_specifiers.len();
            declaration_specifiers.push(Vec::new());
            for specifier in specifiers {
                match specifier {
                    ImportDeclarationSpecifier::ImportSpecifier(spec) => {
                        // Explicit type-only specifiers (import { type X }) are always elided
                        if spec.import_kind.is_type() {
                            type_only_specifiers.insert(spec.local.name.clone().into());
                            continue;
                        }

                        let name: Ident<'a> = spec.local.name.clone().into();
                        value_reference_counts
                            .insert(name, Self::value_reference_count_of(&spec.local, semantic));

                        // Check if this import has only type references
                        if Self::is_type_only_import(&spec.local, semantic) {
                            type_only_specifiers.insert(name.clone());
                        }
                        // Check if this import is only used in decorators compilation removes
                        else if !removed_references.is_empty()
                            && Self::value_references_only_among(
                                &spec.local,
                                semantic,
                                &removed_references,
                            )
                        {
                            type_only_specifiers.insert(name.clone());
                        }
                    }
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) => {
                        let name: Ident<'a> = spec.local.name.clone().into();
                        value_reference_counts
                            .insert(name, Self::value_reference_count_of(&spec.local, semantic));

                        if Self::is_type_only_import(&spec.local, semantic) {
                            type_only_specifiers.insert(name.clone());
                        }
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(spec) => {
                        // Check if the namespace import is type-only using semantic analysis.
                        // e.g., `import * as moment from 'moment'` where `moment` is only
                        // used in type annotations like `moment.Moment`.
                        if Self::is_type_only_import(&spec.local, semantic) {
                            type_only_specifiers.insert(spec.local.name.clone().into());
                        }
                    }
                }
                let name = specifier.local().name;
                declaration_specifiers[declaration].push(name);
                declaration_of.insert(name, declaration);
            }
        }

        // Post-pass: remove identifiers used as computed property keys in type annotations.
        // These are value references (they compute runtime property names) even though they
        // appear inside type contexts. TypeScript preserves these imports.
        // Example: `[fromEmail]: Emailer[]` in a type literal uses `fromEmail` as a value.
        let computed_key_idents = Self::collect_computed_property_key_idents(program);
        for name in &computed_key_idents {
            type_only_specifiers.remove(name);
        }
        let mut kept_by_output = computed_key_idents;

        // Post-pass: when class metadata is emitted, a default import used as a
        // bare ctor-param type reference is emitted into the metadata's
        // `ctorParameters` `type:` by its local name — ngtsc keeps the import
        // via `defaultImportStatement` (`type_to_value.ts` +
        // `attachDefaultImportDeclaration`). Keep it here too, or the emitted
        // `type: Foo` would dangle.
        if emit_metadata {
            let ctor_type_names = Self::collect_ctor_param_type_leftmost_names(program, &consts);
            for name in &ctor_type_names {
                type_only_specifiers.remove(name);
            }
            kept_by_output.extend(ctor_type_names);
        }

        Self {
            type_only_specifiers,
            value_reference_counts,
            kept_by_output,
            declaration_specifiers,
            declaration_of,
        }
    }

    /// Local names of **default imports** that a decorated class's constructor
    /// parameter annotation references bare (`x: Foo` after `T | null`
    /// narrowing, not shadowed by a class type parameter) — the names
    /// `ɵsetClassMetadata`'s `type:` emits verbatim.
    ///
    /// Only default imports need this: named/namespace imports emit
    /// `i{reg}.Export`, which re-anchors the module import, while a default
    /// import emits its local name (upstream's LOCAL + defaultImportStatement).
    fn collect_ctor_param_type_leftmost_names(
        program: &'a Program<'a>,
        consts: &crate::directive::StringConsts<'a>,
    ) -> FxHashSet<Ident<'a>> {
        let mut default_imports: FxHashSet<&'a str> = FxHashSet::default();
        for stmt in &program.body {
            let Statement::ImportDeclaration(import_decl) = stmt else {
                continue;
            };
            // `import type Foo from 'm'` never becomes a value reference.
            if import_decl.import_kind.is_type() {
                continue;
            }
            if let Some(specifiers) = &import_decl.specifiers {
                for specifier in specifiers {
                    if let ImportDeclarationSpecifier::ImportDefaultSpecifier(spec) = specifier {
                        default_imports.insert(spec.local.name.as_str());
                    }
                }
            }
        }
        if default_imports.is_empty() {
            return FxHashSet::default();
        }

        let mut names = FxHashSet::default();
        for stmt in &program.body {
            match stmt {
                Statement::ClassDeclaration(class) => {
                    Self::collect_ctor_type_uses_from_class(
                        class,
                        &default_imports,
                        consts,
                        &mut names,
                    );
                }
                Statement::ExportDefaultDeclaration(export) => {
                    if let oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) =
                        &export.declaration
                    {
                        Self::collect_ctor_type_uses_from_class(
                            class,
                            &default_imports,
                            consts,
                            &mut names,
                        );
                    }
                }
                Statement::ExportDeclaration(export) => {
                    if let oxc_ast::ast::Declaration::ClassDeclaration(class) = &export.declaration
                    {
                        Self::collect_ctor_type_uses_from_class(
                            class,
                            &default_imports,
                            consts,
                            &mut names,
                        );
                    }
                }
                _ => {}
            }
        }
        names
    }

    /// For a class with an `@angular/core` decorator (the only classes that
    /// get `ɵsetClassMetadata`), collect ctor-param type names that hit a
    /// default import — matching `build_param_type_expression`'s decision to
    /// emit the local name.
    fn collect_ctor_type_uses_from_class(
        class: &'a oxc_ast::ast::Class<'a>,
        default_imports: &FxHashSet<&'a str>,
        consts: &crate::directive::StringConsts<'a>,
        names: &mut FxHashSet<Ident<'a>>,
    ) {
        let has_angular_decorator = class
            .decorators
            .iter()
            .any(|d| crate::directive::is_angular_core_decorator(d, Some(consts)));
        if !has_angular_decorator {
            return;
        }
        let type_params = crate::class_metadata::type_param_names_of(class);
        for element in &class.body.body {
            let ClassElement::MethodDefinition(method) = element else {
                continue;
            };
            if method.kind != MethodDefinitionKind::Constructor {
                continue;
            }
            for param in &method.value.params.items {
                let Some(annotation) = &param.type_annotation else {
                    continue;
                };
                let Some(type_ref) =
                    crate::class_metadata::param_type_reference(&annotation.type_annotation)
                else {
                    continue;
                };
                let Some(path) = crate::class_metadata::entity_path(&type_ref.type_name) else {
                    continue;
                };
                let Some(leftmost) = path.first() else {
                    continue;
                };
                if !type_params.contains(leftmost) && default_imports.contains(leftmost.as_str()) {
                    names.insert(*leftmost);
                }
            }
        }
    }

    /// Collect identifiers used as computed property keys in type annotations.
    ///
    /// Computed property keys like `[fromEmail]` in type literals reference runtime values,
    /// even when they appear in type contexts. TypeScript considers these as value references
    /// and preserves their imports.
    fn collect_computed_property_key_idents(program: &'a Program<'a>) -> FxHashSet<Ident<'a>> {
        let mut result = FxHashSet::default();

        for stmt in &program.body {
            Self::collect_computed_keys_from_statement(stmt, &mut result);
        }

        result
    }

    /// Walk a statement collecting computed property key identifiers from type annotations.
    fn collect_computed_keys_from_statement(
        stmt: &'a Statement<'a>,
        result: &mut FxHashSet<Ident<'a>>,
    ) {
        match stmt {
            Statement::ClassDeclaration(class) => {
                Self::collect_computed_keys_from_class(class, result);
            }
            Statement::ExportDefaultDeclaration(export) => {
                if let oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) =
                    &export.declaration
                {
                    Self::collect_computed_keys_from_class(class, result);
                }
            }
            Statement::ExportDeclaration(export) => {
                if let oxc_ast::ast::Declaration::ClassDeclaration(class) = &export.declaration {
                    Self::collect_computed_keys_from_class(class, result);
                }
            }
            _ => {}
        }
    }

    /// Walk class members collecting computed property key identifiers from type annotations.
    fn collect_computed_keys_from_class(
        class: &'a oxc_ast::ast::Class<'a>,
        result: &mut FxHashSet<Ident<'a>>,
    ) {
        for element in &class.body.body {
            if let ClassElement::PropertyDefinition(prop) = element {
                // Check the type annotation for computed property keys in type literals
                if let Some(ts_type) = &prop.type_annotation {
                    Self::collect_computed_keys_from_ts_type(&ts_type.type_annotation, result);
                }
            }
        }
    }

    /// Recursively walk a TypeScript type collecting computed property key identifiers.
    fn collect_computed_keys_from_ts_type(
        ts_type: &'a TSType<'a>,
        result: &mut FxHashSet<Ident<'a>>,
    ) {
        match ts_type {
            TSType::TSTypeLiteral(type_lit) => {
                for member in &type_lit.members {
                    if let oxc_ast::ast::TSSignature::TSPropertySignature(prop_sig) = member {
                        // Check if the property key is computed
                        if prop_sig.computed {
                            Self::collect_idents_from_expr(&prop_sig.key, result);
                        }
                        // Recurse into the property's type annotation to find
                        // computed keys in nested type literals
                        if let Some(type_ann) = &prop_sig.type_annotation {
                            Self::collect_computed_keys_from_ts_type(
                                &type_ann.type_annotation,
                                result,
                            );
                        }
                    }
                }
            }
            TSType::TSUnionType(union_type) => {
                for ty in &union_type.types {
                    Self::collect_computed_keys_from_ts_type(ty, result);
                }
            }
            TSType::TSIntersectionType(intersection_type) => {
                for ty in &intersection_type.types {
                    Self::collect_computed_keys_from_ts_type(ty, result);
                }
            }
            TSType::TSArrayType(array_type) => {
                Self::collect_computed_keys_from_ts_type(&array_type.element_type, result);
            }
            TSType::TSTupleType(tuple_type) => {
                // `TSTupleElement` inherits the full set of `TSType`
                // variants AND adds three of its own: `TSOptionalType`
                // (`[number?]`), `TSRestType` (`[...string[]]`), and
                // `TSNamedTupleMember` (`[name: string]`). The previous
                // `element.to_ts_type()` call panicked with
                // `Option::unwrap() on a None value` whenever a tuple
                // contained any of those three variants — common in
                // real code, e.g. function signatures expressed as
                // tuple types in component decorator metadata.
                //
                // `TSTupleElement` adds `TSOptionalType` and `TSRestType` on top of the
                // inherited `TSType` variants. Use `as_ts_type()` for the common inherited
                // path and unpack the two named variants explicitly.
                // `TSNamedTupleMember` is an inherited `TSType` variant and is handled in
                // the `TSType::TSNamedTupleMember` arm of `collect_computed_keys_from_ts_type`.
                for element in &tuple_type.element_types {
                    if let Some(ty) = element.as_ts_type() {
                        Self::collect_computed_keys_from_ts_type(ty, result);
                        continue;
                    }
                    match element {
                        oxc_ast::ast::TSTupleElement::TSOptionalType(opt) => {
                            Self::collect_computed_keys_from_ts_type(&opt.type_annotation, result);
                        }
                        oxc_ast::ast::TSTupleElement::TSRestType(rest) => {
                            Self::collect_computed_keys_from_ts_type(&rest.type_annotation, result);
                        }
                        _ => {}
                    }
                }
            }
            TSType::TSTypeReference(type_ref) => {
                if let Some(type_args) = &type_ref.type_arguments {
                    for ty in &type_args.params {
                        Self::collect_computed_keys_from_ts_type(ty, result);
                    }
                }
            }
            TSType::TSNamedTupleMember(named) => {
                if let Some(inner) = named.element_type.as_ts_type() {
                    Self::collect_computed_keys_from_ts_type(inner, result);
                }
            }
            TSType::TSParenthesizedType(paren_type) => {
                Self::collect_computed_keys_from_ts_type(&paren_type.type_annotation, result);
            }
            _ => {}
        }
    }

    /// Collect identifier names from an expression (for computed property keys).
    fn collect_idents_from_expr(
        expr: &'a oxc_ast::ast::PropertyKey<'a>,
        result: &mut FxHashSet<Ident<'a>>,
    ) {
        match expr {
            oxc_ast::ast::PropertyKey::StaticIdentifier(_) => {
                // Static identifiers are NOT computed property keys
            }
            oxc_ast::ast::PropertyKey::PrivateIdentifier(_) => {}
            _ => {
                if let Some(expr) = expr.as_expression() {
                    Self::collect_idents_from_expression(expr, result);
                }
            }
        }
    }

    /// Collect identifier names from an expression.
    fn collect_idents_from_expression(expr: &'a Expression<'a>, result: &mut FxHashSet<Ident<'a>>) {
        match expr {
            Expression::Identifier(id) => {
                result.insert(id.name.clone().into());
            }
            Expression::StaticMemberExpression(member) => {
                // For `RecipientType.To`, collect `RecipientType`
                if let Expression::Identifier(id) = &member.object {
                    result.insert(id.name.clone().into());
                }
            }
            _ => {}
        }
    }

    /// Check if an import binding is only used in type positions.
    fn is_type_only_import(id: &BindingIdentifier<'a>, semantic: &Semantic<'a>) -> bool {
        let symbol_id = id.symbol_id();

        // If the symbol has a value redeclaration that shadows the import, it's safe to remove
        // e.g., `import T from 'mod'; const T = 1;`
        let symbol_flags = semantic.scoping().symbol_flags(symbol_id);
        if (symbol_flags - SymbolFlags::Import).is_value() {
            return true; // Shadowed, safe to remove
        }

        // Check if all references to this symbol are type-only
        // If ANY reference is NOT type-only, we must keep the import
        let has_value_reference = semantic
            .scoping()
            .get_resolved_references(symbol_id)
            .any(|reference| !reference.is_type());

        !has_value_reference
    }

    /// Whether every value reference to an import binding is one of `references`
    fn value_references_only_among(
        id: &BindingIdentifier<'a>,
        semantic: &Semantic<'a>,
        references: &FxHashSet<ReferenceId>,
    ) -> bool {
        let scoping = semantic.scoping();
        scoping.get_resolved_reference_ids(id.symbol_id()).iter().all(|&reference| {
            scoping.get_reference(reference).is_type() || references.contains(&reference)
        })
    }

    /// The references the compiled output no longer has, because the transform strips the decorator that held
    /// them and compiles nothing that names them. Only on a class the transform compiles (one with an Angular
    /// class decorator, see [`crate::directive::angular_class_decorator`]):
    ///
    /// - an Angular constructor parameter decorator of the first constructor (the one
    ///   `collect_constructor_decorator_spans` strips and the factory reads), with the argument of `@Inject(X)`
    ///   when it is a bare identifier: the factory names `X` through a namespace import (`i1.X`), while any other
    ///   argument (`TOKEN.sub`, `forwardRef(() => TOKEN)`) is copied as written
    /// - the Angular member decorator itself on a `declare` property, not its arguments: a query predicate
    ///   (`viewQuery(Foo)`) or an input `transform` is compiled into the definition by its local name
    ///
    /// A decorator that stays in the output (not Angular's, or on a class that isn't compiled) keeps all of its
    /// references. `transform` also keeps the imports of decorators it didn't strip after all (a component whose
    /// template is missing).
    fn removed_decorator_references(
        program: &'a Program<'a>,
        consts: &crate::directive::StringConsts<'a>,
    ) -> FxHashSet<ReferenceId> {
        #[derive(Default)]
        struct References(FxHashSet<ReferenceId>);
        impl<'a> Visit<'a> for References {
            fn visit_identifier_reference(&mut self, it: &oxc_ast::ast::IdentifierReference<'a>) {
                if let Some(reference) = it.reference_id.get() {
                    self.0.insert(reference);
                }
            }
        }
        impl References {
            /// The decorator's callee: `Inject` in `@Inject(TOKEN)`, `core` in `@core.Optional()`
            fn visit_callee<'a>(&mut self, decorator: &oxc_ast::ast::Decorator<'a>) {
                match &decorator.expression {
                    Expression::CallExpression(call) => self.visit_expression(&call.callee),
                    expression => self.visit_expression(expression),
                }
            }
        }
        let mut references = References::default();
        for stmt in &program.body {
            let class = match stmt {
                Statement::ClassDeclaration(class) => class,
                Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                    oxc_ast::ast::ExportDefaultDeclarationKind::ClassDeclaration(class) => class,
                    _ => continue,
                },
                Statement::ExportDeclaration(export) => match &export.declaration {
                    oxc_ast::ast::Declaration::ClassDeclaration(class) => class,
                    _ => continue,
                },
                _ => continue,
            };
            if !class.decorators.iter().any(|decorator| {
                crate::directive::angular_class_decorator(decorator, consts).is_some()
            }) {
                continue;
            }
            let constructor = class.body.body.iter().find_map(|element| match element {
                ClassElement::MethodDefinition(method)
                    if method.kind == MethodDefinitionKind::Constructor =>
                {
                    Some(method)
                }
                _ => None,
            });
            for param in constructor.into_iter().flat_map(|ctor| &ctor.value.params.items) {
                for decorator in &param.decorators {
                    // by what it resolves to: `@I(TOKEN)` for `import { Inject as I }` is removed too
                    let Some(name) =
                        crate::directive::angular_param_decorator(decorator, Some(consts))
                    else {
                        continue;
                    };
                    references.visit_callee(decorator);
                    if name == "Inject"
                        && let Expression::CallExpression(call) = &decorator.expression
                        && let Some(oxc_ast::ast::Argument::Identifier(token)) =
                            call.arguments.first()
                    {
                        references.visit_identifier_reference(token);
                    }
                }
            }
            for element in &class.body.body {
                if let ClassElement::PropertyDefinition(prop) = element
                    && prop.declare
                {
                    for decorator in &prop.decorators {
                        if crate::directive::angular_member_decorator(decorator, Some(consts))
                            .is_some()
                        {
                            references.visit_callee(decorator);
                        }
                    }
                }
            }
        }
        references.0
    }

    /// The number of value references to an import binding, or `usize::MAX`
    /// when a value redeclaration shadows it (a reference can't be told apart).
    fn value_reference_count_of(id: &BindingIdentifier<'a>, semantic: &Semantic<'a>) -> usize {
        let symbol_id = id.symbol_id();
        let symbol_flags = semantic.scoping().symbol_flags(symbol_id);
        if (symbol_flags - SymbolFlags::Import).is_value() {
            return usize::MAX;
        }
        semantic
            .scoping()
            .get_resolved_references(symbol_id)
            .filter(|reference| !reference.is_type())
            .count()
    }

    /// How many value (non-type) references the source makes to the named or
    /// default import `name`, or `None` for a name no such import binds.
    pub(crate) fn value_reference_count(&self, name: &str) -> Option<usize> {
        self.value_reference_counts.get(&Ident::from(name)).copied()
    }

    /// Whether the compiled output references `name` as a value although the
    /// source doesn't, like a `jit: true` class's `ctorParameters`.
    pub(crate) fn is_kept_by_output(&self, name: &str) -> bool {
        self.kept_by_output.contains(&Ident::from(name))
    }

    /// The local names of every non-`type` specifier in the import
    /// declaration that binds `name`, `name` included (empty if none does).
    pub(crate) fn declaration_specifiers(&self, name: &str) -> &[Ident<'a>] {
        self.declaration_of
            .get(&Ident::from(name))
            .map_or(&[], |&declaration| &self.declaration_specifiers[declaration])
    }

    /// Remove `name`'s import: the compiled output no longer references it,
    /// such as a component that `@defer` now loads with a dynamic `import()`.
    /// A name the output still needs (see [`Self::is_kept_by_output`]) stays.
    pub(crate) fn elide(&mut self, name: Ident<'a>) {
        if !self.kept_by_output.contains(&name) {
            self.type_only_specifiers.insert(name);
        }
    }

    /// Check if a specifier name should be elided (removed).
    pub fn should_elide(&self, name: &str) -> bool {
        self.type_only_specifiers.contains(&Ident::from(name))
    }

    /// Keep `name`'s import even though the source only references it in
    /// positions the analysis marks elidable.
    ///
    /// `jit: true` classes are downleveled like JIT-mode output: constructor
    /// parameter decorators and types move into `ctorParameters`/`__param`,
    /// so they remain value references in the emitted code even though the
    /// source position made them look removable. Upstream sees the same
    /// result because TypeScript's import elision runs after ngtsc's
    /// transforms, on the rewritten AST.
    pub fn preserve(&mut self, name: &'a str) {
        self.type_only_specifiers.remove(&Ident::from(name));
        self.kept_by_output.insert(Ident::from(name));
    }

    /// Get the set of type-only specifier names.
    pub fn type_only_specifiers(&self) -> &FxHashSet<Ident<'a>> {
        &self.type_only_specifiers
    }

    /// Check if the analyzer found any type-only imports.
    pub fn has_type_only_imports(&self) -> bool {
        !self.type_only_specifiers.is_empty()
    }

    /// Analyze with optional cross-file resolution.
    ///
    /// This method extends the basic analysis with cross-file type resolution.
    /// It resolves import paths to actual files and checks if the exports are
    /// type-only (interfaces, type aliases) in their source files.
    ///
    /// # Arguments
    ///
    /// * `program` - The parsed program to analyze
    /// * `file_path` - The path to the file being analyzed
    /// * `cross_file_analyzer` - The cross-file analyzer instance
    ///
    /// # Note
    ///
    /// This is intended for compare tests only. In production, bundlers like
    /// rolldown handle import elision as part of their tree-shaking process.
    #[cfg(feature = "cross_file_elision")]
    pub fn analyze_with_cross_file(
        program: &'a Program<'a>,
        file_path: &Path,
        cross_file_analyzer: &mut super::cross_file_elision::CrossFileAnalyzer,
    ) -> Self {
        // Compare-test helper: no `setClassMetadata` context is available, so
        // analyze as if no class metadata is emitted.
        let mut analyzer = Self::analyze(program, false);

        // Enhanced analysis: check cross-file for remaining imports
        for stmt in &program.body {
            let Statement::ImportDeclaration(import_decl) = stmt else {
                continue;
            };

            // Skip type-only imports (already handled)
            if import_decl.import_kind.is_type() {
                continue;
            }

            // A source with a lone surrogate (`'\uD800'`) can't be resolved.
            let Some(source) = import_decl.source.value.as_str() else {
                continue;
            };
            let Some(specifiers) = &import_decl.specifiers else {
                continue;
            };

            for specifier in specifiers {
                if let ImportDeclarationSpecifier::ImportSpecifier(spec) = specifier {
                    // Skip explicit type-only specifiers
                    if spec.import_kind.is_type() {
                        continue;
                    }

                    let local_name = &spec.local.name;

                    // Skip if already marked as type-only by semantic analysis
                    if analyzer.type_only_specifiers.contains(local_name) {
                        continue;
                    }

                    // Check cross-file: is the export type-only in the source file?
                    let imported_name = spec.imported.name().as_str();
                    if cross_file_analyzer.is_type_only_import(source, imported_name, file_path) {
                        analyzer.type_only_specifiers.insert(local_name.clone().into());
                    }
                }
            }
        }

        analyzer
    }
}

/// Compute import elision edits as `Vec<Edit>` objects.
///
/// Returns edits that remove type-only import specifiers from the source.
/// Entire import declarations are removed if all their specifiers are type-only,
/// or if the import has no specifiers at all (`import {} from 'module'`).
/// Type-only export declarations (`export type { X }`, `export { type X }`) are also removed.
pub fn import_elision_edits<'a>(
    source: &str,
    program: &Program<'a>,
    analyzer: &ImportElisionAnalyzer<'a>,
) -> Vec<Edit> {
    // Check if there are empty imports that need removal (import {} from '...')
    let has_empty_imports = program.body.iter().any(|stmt| {
        if let Statement::ImportDeclaration(import_decl) = stmt {
            if let Some(specifiers) = &import_decl.specifiers {
                return specifiers.is_empty();
            }
        }
        false
    });

    // Check if there are type-only exports that need removal.
    // ExportDeclaration always has a declaration (export class/const/type Foo) — those are
    // not elided here. Only ExportNamedDeclaration (local `{ }`) and ExportFromDeclaration
    // (re-exports) carry type-only / per-specifier type elision.
    let has_type_only_exports = program.body.iter().any(|stmt| match stmt {
        Statement::ExportNamedDeclaration(export_decl) => {
            export_decl.export_kind.is_type()
                || export_decl.specifiers.iter().any(|spec| spec.export_kind.is_type())
        }
        Statement::ExportFromDeclaration(export_decl) => {
            export_decl.export_kind.is_type()
                || export_decl.specifiers.iter().any(|spec| spec.export_kind.is_type())
        }
        _ => false,
    });

    if !analyzer.has_type_only_imports() && !has_empty_imports && !has_type_only_exports {
        return Vec::new();
    }

    let mut edits: Vec<Edit> = Vec::new();

    for stmt in &program.body {
        let oxc_ast::ast::Statement::ImportDeclaration(import_decl) = stmt else {
            continue;
        };

        // Skip type-only imports (already handled by TS stripping)
        if import_decl.import_kind.is_type() {
            continue;
        }

        let Some(specifiers) = &import_decl.specifiers else {
            continue;
        };

        // Count how many specifiers should be kept vs removed
        let (kept, removed): (Vec<_>, Vec<_>) = specifiers.iter().partition(|spec| {
            let name = match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local.name,
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local.name,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local.name,
            };
            !analyzer.should_elide(name.as_str())
        });

        if removed.is_empty() && !kept.is_empty() {
            // All specifiers kept and at least one exists, no changes needed
            continue;
        }

        if kept.is_empty() {
            // All specifiers removed - remove entire import declaration
            let start = import_decl.span.start as usize;
            let mut end = import_decl.span.end as usize;

            // Also remove trailing newline
            let bytes = source.as_bytes();
            while end < bytes.len() && (bytes[end] == b'\n' || bytes[end] == b'\r') {
                end += 1;
            }

            edits.push(Edit::delete(start as u32, end as u32));
        } else {
            // Partial removal - reconstruct import with only kept specifiers

            let mut default_import: Option<&str> = None;
            let mut named_specifiers: Vec<String> = Vec::new();

            for spec in &kept {
                match spec {
                    ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                        default_import = Some(s.local.name.as_str());
                    }
                    ImportDeclarationSpecifier::ImportSpecifier(s) => {
                        let imported_name = s.imported.name().as_str();
                        let local_name = s.local.name.as_str();
                        if imported_name == local_name {
                            named_specifiers.push(local_name.to_string());
                        } else {
                            named_specifiers.push(format!("{imported_name} as {local_name}"));
                        }
                    }
                    ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                        named_specifiers.push(format!("* as {}", s.local.name));
                    }
                }
            }

            // Build the new import statement
            let mut new_import = String::from("import ");

            if let Some(default_name) = default_import {
                new_import.push_str(default_name);
                if !named_specifiers.is_empty() {
                    new_import.push_str(", ");
                }
            }

            if !named_specifiers.is_empty() {
                if named_specifiers.len() == 1 && named_specifiers[0].starts_with("* as ") {
                    new_import.push_str(&named_specifiers[0]);
                } else {
                    new_import.push_str("{ ");
                    new_import.push_str(&named_specifiers.join(", "));
                    new_import.push_str(" }");
                }
            }

            new_import.push_str(" from ");
            push_module_specifier(&mut new_import, &import_decl.source, source);
            new_import.push(';');

            // Replace the entire import declaration
            let start = import_decl.span.start as usize;
            let mut end = import_decl.span.end as usize;

            // Include trailing newline in replacement
            let bytes = source.as_bytes();
            while end < bytes.len() && (bytes[end] == b'\n' || bytes[end] == b'\r') {
                end += 1;
                if !new_import.ends_with('\n') {
                    new_import.push('\n');
                }
            }

            edits.push(Edit::replace(start as u32, end as u32, new_import));
        }
    }

    // Process type-only export declarations.
    // Local `export { ... }` → ExportNamedDeclaration; re-exports → ExportFromDeclaration.
    // `export class/const/...` is ExportDeclaration and is skipped (no type elision here).
    for stmt in &program.body {
        let (span_start, span_end, export_kind_is_type, specifiers, source_module) = match stmt {
            Statement::ExportNamedDeclaration(export_decl) => (
                export_decl.span.start,
                export_decl.span.end,
                export_decl.export_kind.is_type(),
                &export_decl.specifiers,
                None,
            ),
            Statement::ExportFromDeclaration(export_decl) => (
                export_decl.span.start,
                export_decl.span.end,
                export_decl.export_kind.is_type(),
                &export_decl.specifiers,
                Some(&export_decl.source),
            ),
            _ => continue,
        };

        if export_kind_is_type {
            // `export type { X }` or `export type { X } from './foo'` — remove entirely
            let start = span_start as usize;
            let mut end = span_end as usize;
            let bytes = source.as_bytes();
            while end < bytes.len() && (bytes[end] == b'\n' || bytes[end] == b'\r') {
                end += 1;
            }
            edits.push(Edit::delete(start as u32, end as u32));
            continue;
        }

        // Check for individual type-only specifiers (`export { type X, Y }`)
        let (type_specs, value_specs): (Vec<_>, Vec<_>) =
            specifiers.iter().partition(|spec| spec.export_kind.is_type());

        if type_specs.is_empty() {
            continue;
        }

        let start = span_start as usize;
        let mut end = span_end as usize;
        let bytes = source.as_bytes();
        while end < bytes.len() && (bytes[end] == b'\n' || bytes[end] == b'\r') {
            end += 1;
        }

        if value_specs.is_empty() {
            // All specifiers are type-only — remove entire statement
            edits.push(Edit::delete(start as u32, end as u32));
        } else {
            // Partial removal — reconstruct with only value specifiers
            let mut named_specifiers: Vec<String> = Vec::new();
            for spec in &value_specs {
                let local_name = spec.local.name().as_str();
                let exported_name = spec.exported.name().as_str();
                if local_name == exported_name {
                    named_specifiers.push(local_name.to_string());
                } else {
                    named_specifiers.push(format!("{local_name} as {exported_name}"));
                }
            }

            let mut new_export = String::from("export { ");
            new_export.push_str(&named_specifiers.join(", "));
            new_export.push_str(" }");

            if let Some(source_mod) = source_module {
                new_export.push_str(" from ");
                push_module_specifier(&mut new_export, source_mod, source);
            }
            new_export.push(';');

            if end > span_end as usize {
                new_export.push('\n');
            }

            edits.push(Edit::replace(start as u32, end as u32, new_export));
        }
    }

    edits
}

/// Write a rebuilt declaration's module specifier: its value in double quotes,
/// or, for a value with a lone surrogate (`'\uD800'`), which has no `&str`,
/// the literal as written.
fn push_module_specifier(out: &mut String, specifier: &StringLiteral<'_>, source: &str) {
    match specifier.value.as_str() {
        Some(value) => {
            out.push('"');
            out.push_str(value);
            out.push('"');
        }
        None => out.push_str(specifier.span.source_text(source)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::optimizer::apply_edits;
    use oxc_allocator::Allocator;
    use oxc_parser::Parser;
    use oxc_span::SourceType;

    fn filter_imports<'a>(
        source: &str,
        program: &Program<'a>,
        analyzer: &ImportElisionAnalyzer<'a>,
    ) -> String {
        let edits = import_elision_edits(source, program, analyzer);
        if edits.is_empty() {
            return source.to_string();
        }
        apply_edits(source, edits)
    }

    fn analyze_source(source: &str) -> FxHashSet<String> {
        analyze_source_with_metadata(source, false)
    }

    fn analyze_source_with_metadata(source: &str, emit_metadata: bool) -> FxHashSet<String> {
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let parser_ret = Parser::new(&allocator, source, source_type).parse();
        let analyzer = ImportElisionAnalyzer::analyze(&parser_ret.program, emit_metadata);
        analyzer.type_only_specifiers().iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn test_type_only_interface_implements() {
        let source = r#"
import { Component, OnInit, OnDestroy } from '@angular/core';

class MyComponent implements OnInit, OnDestroy {
    ngOnInit() {}
    ngOnDestroy() {}
}
"#;
        let type_only = analyze_source(source);
        // OnInit and OnDestroy are only used in implements clause (type position)
        assert!(type_only.contains("OnInit"));
        assert!(type_only.contains("OnDestroy"));
        // Component is not used at all, so it's also type-only
        assert!(type_only.contains("Component"));
    }

    #[test]
    fn test_value_usage_in_decorator() {
        let source = r#"
import { Component, Input } from '@angular/core';

@Component({ selector: 'app-test' })
class MyComponent {
    @Input() value: string;
}
"#;
        let type_only = analyze_source(source);
        // Component and Input are used in decorators (value position)
        assert!(!type_only.contains("Component"));
        assert!(!type_only.contains("Input"));
    }

    #[test]
    fn test_type_annotation_only() {
        let source = r#"
import { UserId } from './types';

function processUser(id: UserId): void {}
"#;
        let type_only = analyze_source(source);
        // UserId is only used in type annotation
        assert!(type_only.contains("UserId"));
    }

    #[test]
    fn test_value_usage_in_expression() {
        let source = r#"
import { AuthService } from './auth.service';

const service = new AuthService();
"#;
        let type_only = analyze_source(source);
        // AuthService is used in a new expression (value position)
        assert!(!type_only.contains("AuthService"));
    }

    fn filter_source(source: &str) -> String {
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let parser_ret = Parser::new(&allocator, source, source_type).parse();
        let analyzer = ImportElisionAnalyzer::analyze(&parser_ret.program, false);
        filter_imports(source, &parser_ret.program, &analyzer)
    }

    #[test]
    fn test_filter_partial_multiline_import() {
        let source = r#"import {
  ChangeDetectorRef,
  Component,
  OnDestroy,
  OnInit,
} from "@angular/core";

@Component({ selector: 'test' })
class MyComponent implements OnInit, OnDestroy {
    constructor(private cdr: ChangeDetectorRef) {}
    ngOnInit() {}
    ngOnDestroy() {}
}
"#;
        let filtered = filter_source(source);
        println!("=== FILTERED OUTPUT ===");
        println!("{}", filtered);
        println!("=== END ===");

        // Extract just the import line
        let import_line = filtered.lines().find(|l| l.starts_with("import")).unwrap();
        println!("Import line: {}", import_line);

        // Should keep Component (used as decorator)
        // Should remove ChangeDetectorRef, OnInit, OnDestroy (all type-only)
        // Constructor parameter types ARE type-only - Angular generates namespace imports for DI
        assert!(import_line.contains("Component"));
        assert!(!import_line.contains("ChangeDetectorRef"));
        assert!(!import_line.contains("OnInit"));
        assert!(!import_line.contains("OnDestroy"));

        // Verify the filtered code is syntactically valid
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let parser_ret = Parser::new(&allocator, &filtered, source_type).parse();
        assert!(
            parser_ret.diagnostics.is_empty(),
            "Filtered source should be valid TypeScript: {:?}",
            parser_ret.diagnostics
        );
    }

    #[test]
    fn test_filter_bitwarden_style() {
        // Mimics the real Bitwarden app.component.ts structure
        let source = r#"// FIXME: comment
// @ts-strict-ignore
import {
  ChangeDetectorRef,
  Component,
  DestroyRef,
  inject,
  NgZone,
  OnDestroy,
  OnInit,
} from "@angular/core";
import { takeUntilDestroyed } from "@angular/core/rxjs-interop";
import { NavigationEnd, Router, RouterOutlet } from "@angular/router";
import {
  catchError,
  filter,
  Subject,
} from "rxjs";
import {
  AuthRequestServiceAbstraction,
  LogoutReason,
  UserDecryptionOptionsServiceAbstraction,
} from "@bitwarden/auth/common";
import { AuthService } from "@bitwarden/common/auth/abstractions/auth.service";

@Component({ selector: 'app-test' })
class AppComponent implements OnInit, OnDestroy {
    private authService = inject(AuthService);

    constructor(
        private cdr: ChangeDetectorRef,
        private router: Router,
        private authRequestService: AuthRequestServiceAbstraction,
    ) {}

    ngOnInit() {}
    ngOnDestroy() {}
}
"#;
        let filtered = filter_source(source);
        println!("=== BITWARDEN STYLE FILTERED OUTPUT ===");
        println!("{}", filtered);
        println!("=== END ===");

        // Verify it's syntactically valid
        let allocator = Allocator::default();
        let source_type = SourceType::ts();
        let parser_ret = Parser::new(&allocator, &filtered, source_type).parse();
        if !parser_ret.diagnostics.is_empty() {
            for err in &parser_ret.diagnostics {
                eprintln!("Parse error: {:?}", err);
            }
        }
        assert!(parser_ret.diagnostics.is_empty(), "Filtered source should be valid TypeScript");

        // Check that type-only imports are removed from the right places
        let import_lines: Vec<_> =
            filtered.lines().filter(|l| l.starts_with("import") || l.contains("from")).collect();
        println!("Import lines:");
        for line in &import_lines {
            println!("  {}", line);
        }
    }

    #[test]
    fn test_view_children_type_elided() {
        // QueryList is a type annotation that gets erased during compilation
        // Angular doesn't import QueryList even when using @ViewChildren
        let source = r#"
import { Component, ViewChildren, QueryList } from '@angular/core';
import { ItemComponent } from './item.component';

@Component({ selector: 'app-test', template: '' })
class MyComponent {
    @ViewChildren(ItemComponent) items: QueryList<ItemComponent>;
}
"#;
        let type_only = analyze_source(source);

        // ItemComponent is passed to @ViewChildren(ItemComponent) - value reference, preserved
        assert!(
            !type_only.contains("ItemComponent"),
            "ItemComponent passed to @ViewChildren decorator should be preserved"
        );

        // QueryList is only used in type annotation - should be elided (matches Angular behavior)
        assert!(
            type_only.contains("QueryList"),
            "QueryList should be elided (type-only, matches Angular behavior)"
        );
    }

    #[test]
    fn test_content_children_type_elided() {
        // QueryList is a type annotation that gets erased during compilation
        // Angular doesn't import QueryList even when using @ContentChildren
        let source = r#"
import { Component, ContentChildren, QueryList } from '@angular/core';
import { TabComponent } from './tab.component';

@Component({ selector: 'app-test', template: '' })
class MyComponent {
    @ContentChildren(TabComponent) tabs: QueryList<TabComponent>;
}
"#;
        let type_only = analyze_source(source);

        // TabComponent is passed to @ContentChildren(TabComponent) - value reference, preserved
        assert!(
            !type_only.contains("TabComponent"),
            "TabComponent passed to @ContentChildren decorator should be preserved"
        );

        // QueryList is only used in type annotation - should be elided (matches Angular behavior)
        assert!(
            type_only.contains("QueryList"),
            "QueryList should be elided (type-only, matches Angular behavior)"
        );
    }

    #[test]
    fn test_content_children_setter_type_elided() {
        // QueryList is a type annotation that gets erased during compilation
        // Angular doesn't import QueryList even when using @ContentChildren with setters
        let source = r#"
import { Component, ContentChildren, QueryList } from '@angular/core';
import { OptionComponent } from './option.component';

@Component({ selector: 'app-test', template: '' })
class MyComponent {
    @ContentChildren(OptionComponent)
    set options(value: QueryList<OptionComponent>) {
        // setter logic
    }
}
"#;
        let type_only = analyze_source(source);

        // OptionComponent is passed to @ContentChildren - value reference, preserved
        assert!(
            !type_only.contains("OptionComponent"),
            "OptionComponent passed to @ContentChildren decorator should be preserved"
        );

        // QueryList is only used in type annotation - should be elided (matches Angular behavior)
        assert!(
            type_only.contains("QueryList"),
            "QueryList should be elided (type-only, matches Angular behavior)"
        );
    }

    #[test]
    fn test_angular_core_classes_with_value_usage() {
        // QueryList has a value usage (new QueryList()) so it should be preserved
        // ElementRef is only in type position so it should be elided (unless used as DI token)
        let source = r#"
import { Component, ViewChildren, QueryList, ElementRef } from '@angular/core';

@Component({ selector: 'app-test', template: '' })
class MyComponent {
    @ViewChildren("uriInput") items: QueryList<ElementRef> = new QueryList();
}
"#;
        let type_only = analyze_source(source);

        // QueryList has value usage (new QueryList()) - preserved
        assert!(
            !type_only.contains("QueryList"),
            "QueryList with value usage (new QueryList()) should be preserved"
        );
        // ElementRef is only in type annotation - should be elided
        assert!(
            type_only.contains("ElementRef"),
            "ElementRef in type-only position should be elided"
        );
    }

    #[test]
    fn test_input_type_annotation_elided() {
        // Types in @Input() type annotations are now elided (pure semantic analysis)
        let source = r#"
import { Component, Input } from '@angular/core';
import { UserId } from './types';
import { Translation } from './i18n';

@Component({ selector: 'app-test' })
class MyComponent {
    @Input() userId?: UserId;
    @Input() text?: Translation;
}
"#;
        let type_only = analyze_source(source);

        // UserId and Translation are ONLY in type annotations - should be elided
        assert!(type_only.contains("UserId"), "UserId in type annotation should be elided");
        assert!(
            type_only.contains("Translation"),
            "Translation in type annotation should be elided"
        );

        // Component and Input are used as decorators - value references, preserved
        assert!(!type_only.contains("Component"));
        assert!(!type_only.contains("Input"));
    }

    #[test]
    fn test_type_annotation_elided() {
        // Types used only in type annotations should be elided
        let source = r#"
import { Component } from '@angular/core';
import { UserId } from './types';

@Component({ selector: 'app-test' })
class MyComponent {
    // No decorator on this property
    userId?: UserId;
}
"#;
        let type_only = analyze_source(source);

        // UserId is in a type annotation - should be elided
        assert!(type_only.contains("UserId"), "UserId in type annotation should be elided");
    }

    #[test]
    fn test_angular_vs_external_types() {
        // Test that types only used in type positions are properly elided
        // regardless of whether they come from @angular/core or external modules
        let source = r#"
import { Component, ViewChildren, QueryList, ElementRef } from '@angular/core';
import { UserId } from '@bitwarden/common';
import { Translation } from './i18n';
import { ItemComponent } from './item.component';

@Component({ selector: 'app-test', template: '' })
class MyComponent {
    @ViewChildren(ItemComponent) items: QueryList<ElementRef>;
    userId?: UserId;
    text?: Translation;
}
"#;
        let type_only = analyze_source(source);

        // QueryList and ElementRef are only in type positions - should be elided
        assert!(
            type_only.contains("QueryList"),
            "QueryList in type-only position should be elided"
        );
        assert!(
            type_only.contains("ElementRef"),
            "ElementRef in type-only position should be elided"
        );

        // ItemComponent is passed to @ViewChildren decorator - value reference
        assert!(
            !type_only.contains("ItemComponent"),
            "ItemComponent passed to decorator should be preserved"
        );

        // External types only used in type annotations - should be elided
        assert!(
            type_only.contains("UserId"),
            "UserId from external module should be elided (type-only)"
        );
        assert!(
            type_only.contains("Translation"),
            "Translation from external module should be elided (type-only)"
        );
    }

    #[test]
    fn test_known_angular_di_classes_preserved() {
        // Verify known Angular classes used for DI are preserved when used as DI tokens
        // Note: QueryList is NOT a DI token - it's a type annotation that gets erased
        let source = r#"
import {
    Component,
    ElementRef,
    TemplateRef,
    ViewContainerRef,
    ChangeDetectorRef,
    Renderer2,
    NgZone,
    Injector,
    ApplicationRef,
    ComponentRef,
    EmbeddedViewRef,
    ViewRef,
    NgModuleRef,
    EnvironmentInjector,
    DestroyRef,
} from '@angular/core';

@Component({ selector: 'app-test' })
class MyComponent {
    // These are only in type positions, so they should be elided
    // unless used as DI tokens in constructor
    b: ElementRef;
    c: TemplateRef<any>;
    d: ViewContainerRef;
    e: ChangeDetectorRef;
    f: Renderer2;
    g: NgZone;
    h: Injector;
    i: ApplicationRef;
    j: ComponentRef<any>;
    k: EmbeddedViewRef<any>;
    l: ViewRef;
    m: NgModuleRef<any>;
    n: EnvironmentInjector;
    o: DestroyRef;
}
"#;
        let type_only = analyze_source(source);

        // All these are only in type positions (not DI tokens), so they should be elided
        let angular_classes = [
            "TemplateRef",
            "ViewContainerRef",
            "ChangeDetectorRef",
            "Renderer2",
            "NgZone",
            "Injector",
            "ApplicationRef",
            "ComponentRef",
            "EmbeddedViewRef",
            "ViewRef",
            "NgModuleRef",
            "EnvironmentInjector",
            "DestroyRef",
        ];

        for class_name in angular_classes {
            assert!(
                type_only.contains(class_name),
                "{class_name} in type-only position should be elided"
            );
        }
    }

    #[test]
    fn test_angular_type_aliases_elided() {
        // Type aliases from @angular/core like Signal, WritableSignal, OutputRef
        // should be elided when only used in type positions
        let source = r#"
import {
    Component,
    Signal,
    WritableSignal,
    OutputRef,
} from '@angular/core';

@Component({ selector: 'app-test' })
class MyComponent {
    a: Signal<any>;
    b: WritableSignal<any>;
    c: OutputRef<any>;
}
"#;
        let type_only = analyze_source(source);

        // Type aliases should be elided (they're not actual classes)
        assert!(type_only.contains("Signal"), "Signal is a type alias and should be elided");
        assert!(
            type_only.contains("WritableSignal"),
            "WritableSignal is a type alias and should be elided"
        );
        assert!(type_only.contains("OutputRef"), "OutputRef is a type alias and should be elided");
    }

    // =========================================================================
    // Constructor Parameter Decorator Elision Tests
    // =========================================================================
    //
    // Constructor parameter decorators like @Inject, @Optional, @Self, @SkipSelf,
    // @Host, and @Attribute are removed by Angular's compiler and converted to
    // factory metadata. Their imports should be elided.
    //
    // Reference: packages/compiler-cli/src/ngtsc/transform/jit/src/downlevel_decorators_transform.ts

    #[test]
    fn test_optional_decorator_elided() {
        // @Optional is a constructor parameter decorator that should be elided
        let source = r#"
import { Component, ElementRef, Optional } from "@angular/core";
import { FormControlComponent } from "./form-control";

@Component({ selector: 'bit-label' })
export class BitLabelComponent {
    constructor(
        private elementRef: ElementRef<HTMLInputElement>,
        @Optional() private parentFormControl: FormControlComponent,
    ) {}
}
"#;
        let type_only = analyze_source(source);

        // Optional should be elided (only used as ctor param decorator)
        assert!(
            type_only.contains("Optional"),
            "Optional decorator should be elided (only used in ctor param)"
        );

        // Component should be preserved (class decorator)
        assert!(
            !type_only.contains("Component"),
            "Component should be preserved (class decorator)"
        );

        // ElementRef and FormControlComponent are type annotations - should be elided
        assert!(type_only.contains("ElementRef"), "ElementRef should be elided (type annotation)");
        assert!(
            type_only.contains("FormControlComponent"),
            "FormControlComponent should be elided (type annotation)"
        );
    }

    #[test]
    fn test_inject_token_used_elsewhere_in_the_file_is_kept() {
        // `@Inject(TOKEN)` is removed, but TOKEN is also a value elsewhere in the file: an exported const, a
        // function, a nested scope. Each needs the import.
        for elsewhere in [
            "export const providers = [{ provide: TOKEN, useValue: 1 }];",
            "export function token() { return TOKEN; }",
            "function token() { return TOKEN; }",
            "export default [TOKEN];",
            "if (true) { console.log(TOKEN); }",
            "export class Other { static token = TOKEN; }",
        ] {
            let source = format!(
                r#"
import {{ Injectable, Inject }} from "@angular/core";
import {{ TOKEN }} from "./tokens";

@Injectable()
export class TestService {{
    constructor(@Inject(TOKEN) private value: unknown) {{}}
}}
{elsewhere}
"#
            );
            let type_only = analyze_source(&source);
            assert!(
                !type_only.contains("TOKEN"),
                "TOKEN must be kept when the file has `{elsewhere}`"
            );
            assert!(type_only.contains("Inject"), "Inject is still elided with `{elsewhere}`");
        }
    }

    #[test]
    fn test_aliased_inject_counts_as_removed() {
        // `@I(TOKEN)` is Angular's Inject under an alias, which compilation removes like `@Inject(TOKEN)`
        let source = r#"
import { Injectable, Inject } from "@angular/core";
import { Inject as I } from "@angular/core";
import { TOKEN } from "./tokens";

@Injectable()
export class A {
    constructor(@I(TOKEN) private value: unknown) {}
}

@Injectable()
export class B {
    constructor(@Inject(TOKEN) private value: unknown) {}
}
"#;
        assert!(analyze_source(source).contains("TOKEN"));
    }

    #[test]
    fn test_inject_decorator_and_token_elided() {
        // @Inject(TOKEN) - both the decorator and the token should be elided
        let source = r#"
import { Component, Inject } from "@angular/core";
import { DOCUMENT } from "@angular/common";

@Component({ selector: 'app-test' })
export class TestComponent {
    constructor(
        @Inject(DOCUMENT) private document: Document,
    ) {}
}
"#;
        let type_only = analyze_source(source);

        // Inject should be elided (ctor param decorator)
        assert!(type_only.contains("Inject"), "Inject decorator should be elided");

        // DOCUMENT token should be elided (only used in @Inject argument)
        assert!(
            type_only.contains("DOCUMENT"),
            "DOCUMENT token should be elided (only used in @Inject)"
        );

        // Component should be preserved
        assert!(!type_only.contains("Component"), "Component should be preserved");
    }

    #[test]
    fn test_multiple_param_decorators_elided() {
        // Multiple parameter decorators should all be elided
        let source = r#"
import { Component, Inject, Optional, Self, SkipSelf, Host } from "@angular/core";
import { LOCALE_ID } from "@angular/core";

@Component({ selector: 'app-test' })
export class TestComponent {
    constructor(
        @Optional() @Self() service1: Service1,
        @Optional() @SkipSelf() service2: Service2,
        @Host() service3: Service3,
        @Inject(LOCALE_ID) locale: string,
    ) {}
}
"#;
        let type_only = analyze_source(source);

        // All param decorators should be elided
        for decorator in ["Inject", "Optional", "Self", "SkipSelf", "Host"] {
            assert!(type_only.contains(decorator), "{} decorator should be elided", decorator);
        }

        // LOCALE_ID token should be elided
        assert!(type_only.contains("LOCALE_ID"), "LOCALE_ID token should be elided");

        // Component should be preserved
        assert!(!type_only.contains("Component"), "Component should be preserved");
    }

    #[test]
    fn test_param_decorators_and_inject_token_kept_when_metadata_emitted() {
        // Same source as test_multiple_param_decorators_elided, but analyzed as
        // if `ɵsetClassMetadata` will be emitted: `ctorParameters` names all of
        // these bare, so none is type-only (issue #520; ngtsc keeps them).
        let source = r#"
import { Component, Inject, Optional, Self, SkipSelf, Host } from "@angular/core";
import { LOCALE_ID } from "@angular/core";

@Component({ selector: 'app-test' })
export class TestComponent {
    constructor(
        @Optional() @Self() service1: Service1,
        @Optional() @SkipSelf() service2: Service2,
        @Host() service3: Service3,
        @Inject(LOCALE_ID) locale: string,
    ) {}
}
"#;
        let type_only = analyze_source_with_metadata(source, true);

        for name in ["Inject", "Optional", "Self", "SkipSelf", "Host", "LOCALE_ID"] {
            assert!(
                !type_only.contains(name),
                "{name} must survive while setClassMetadata is emitted"
            );
        }

        // Type-only imports are still elided under metadata emission: the
        // parameter type is referenced via a namespace import (`i1.MyService`),
        // not by its bare name.
        let source2 = r#"
import { Component, Optional } from "@angular/core";
import { MyService } from "./service";

@Component({ selector: 'app-test' })
export class TestComponent {
    constructor(@Optional() service: MyService) {}
}
"#;
        let type_only2 = analyze_source_with_metadata(source2, true);
        assert!(!type_only2.contains("Optional"), "Optional must survive");
        assert!(type_only2.contains("MyService"), "MyService is type-only — still elided");
    }

    #[test]
    fn test_inject_token_used_elsewhere_preserved() {
        // If a token is used outside @Inject, it should be preserved
        let source = r#"
import { Component, Inject } from "@angular/core";
import { DOCUMENT } from "@angular/common";

@Component({ selector: 'app-test' })
export class TestComponent {
    doc = DOCUMENT;

    constructor(
        @Inject(DOCUMENT) private document: Document,
    ) {}
}
"#;
        let type_only = analyze_source(source);

        // Inject should still be elided
        assert!(type_only.contains("Inject"), "Inject decorator should be elided");

        // DOCUMENT should NOT be elided (used in property initializer)
        assert!(!type_only.contains("DOCUMENT"), "DOCUMENT should NOT be elided (used as value)");
    }

    #[test]
    fn test_optional_used_elsewhere_preserved() {
        // If Optional is used somewhere other than ctor param, preserve it
        // This is a contrived example but tests the logic
        let source = r#"
import { Component, Optional } from "@angular/core";

const opt = Optional;

@Component({ selector: 'app-test' })
export class TestComponent {
    constructor(
        @Optional() private service: SomeService,
    ) {}
}
"#;
        let type_only = analyze_source(source);

        // Optional should NOT be elided (used in value position)
        assert!(!type_only.contains("Optional"), "Optional should NOT be elided (used as value)");
    }

    #[test]
    fn test_attribute_decorator_elided() {
        // @Attribute is also a parameter decorator that should be elided
        let source = r#"
import { Component, Attribute } from "@angular/core";

@Component({ selector: 'app-test' })
export class TestComponent {
    constructor(
        @Attribute('name') private name: string,
    ) {}
}
"#;
        let type_only = analyze_source(source);

        // Attribute should be elided
        assert!(type_only.contains("Attribute"), "Attribute decorator should be elided");
    }

    #[test]
    fn test_bitwarden_bit_label_component() {
        // Real-world example from Bitwarden codebase
        let source = r#"
import { Component, ElementRef, HostBinding, Input, Optional, input } from "@angular/core";
import { FormControlComponent } from "../form-control/form-control.component";

@Component({
  selector: "bit-label",
  templateUrl: "./label.component.html",
  host: {
    class: "tw-block",
  },
})
export class BitLabelComponent {
  @HostBinding("class") get classList() { return ""; }
  @Input() for?: string;
  readonly required = input(false);

  constructor(
    private elementRef: ElementRef<HTMLInputElement>,
    @Optional() private parentFormControl: FormControlComponent,
  ) {}
}
"#;
        let type_only = analyze_source(source);

        // Optional should be elided (ctor param decorator)
        assert!(type_only.contains("Optional"), "Optional should be elided");

        // ElementRef and FormControlComponent are type annotations - should be elided
        assert!(type_only.contains("ElementRef"), "ElementRef should be elided (type annotation)");
        assert!(
            type_only.contains("FormControlComponent"),
            "FormControlComponent should be elided (type annotation)"
        );

        // Component, HostBinding, Input, input should be preserved (runtime decorators/values)
        assert!(!type_only.contains("Component"), "Component should be preserved");
        assert!(!type_only.contains("HostBinding"), "HostBinding should be preserved");
        assert!(!type_only.contains("Input"), "Input should be preserved");
        assert!(!type_only.contains("input"), "input function should be preserved");
    }

    #[test]
    fn test_filter_optional_decorator_from_imports() {
        // Test the filter function removes Optional from imports
        let source = r#"import { Component, ElementRef, HostBinding, Input, Optional, input } from "@angular/core";
import { FormControlComponent } from "./form-control";

@Component({ selector: 'bit-label' })
export class BitLabelComponent {
    @HostBinding("class") get classList() { return ""; }
    @Input() for?: string;
    readonly required = input(false);

    constructor(
        private elementRef: ElementRef<HTMLInputElement>,
        @Optional() private parentFormControl: FormControlComponent,
    ) {}
}
"#;
        let filtered = filter_source(source);
        println!("=== FILTERED OUTPUT ===");
        println!("{}", filtered);
        println!("=== END ===");

        // Extract just the first import line
        let import_line = filtered
            .lines()
            .find(|l| l.starts_with("import") && l.contains("@angular/core"))
            .unwrap();
        println!("Import line: {}", import_line);

        // Should NOT contain Optional
        assert!(
            !import_line.contains("Optional"),
            "Optional should be removed from imports. Import line: {}",
            import_line
        );

        // Should still contain Component, HostBinding, Input, input
        assert!(import_line.contains("Component"), "Component should be in imports");
        assert!(import_line.contains("HostBinding"), "HostBinding should be in imports");
        assert!(import_line.contains("Input"), "Input should be in imports");
        assert!(import_line.contains("input"), "input should be in imports");

        // Should NOT contain ElementRef (type annotation)
        assert!(!import_line.contains("ElementRef"), "ElementRef should be removed from imports");
    }

    // =========================================================================
    // Regression tests for specific import specifier mismatches (1d category)
    // =========================================================================

    #[test]
    fn test_inline_type_specifier_elided() {
        // Case 1: `import { type FormGroup, FormControl } from '@angular/forms'`
        // The `type` keyword on a specifier means it should be elided.
        // Angular strips type-only specifiers regardless of other specifiers in the same import.
        let source = r#"
import { type FormGroup, FormControl, FormsModule, ReactiveFormsModule, UntypedFormGroup, Validators } from '@angular/forms';
import { Component } from '@angular/core';

@Component({ selector: 'test' })
class MyComponent {
    form = new FormControl('');
    addEditFieldForm = new UntypedFormGroup({});
}

type AddEditFieldForm = FormGroup<{ name: FormControl<string> }>;
"#;
        let type_only = analyze_source(source);

        // FormGroup has `type` keyword — must be elided
        assert!(
            type_only.contains("FormGroup"),
            "type FormGroup (inline type specifier) should be elided"
        );

        // FormControl, UntypedFormGroup are used as values — must NOT be elided
        assert!(
            !type_only.contains("FormControl"),
            "FormControl should be preserved (value usage)"
        );
        assert!(
            !type_only.contains("UntypedFormGroup"),
            "UntypedFormGroup should be preserved (value usage)"
        );
    }

    #[test]
    fn test_declare_property_viewchild_arg_kept() {
        // Case 2: `@ViewChild(GridstackComponent) declare gridstack: GridstackComponent;`
        // The decorator is stripped, but the query is compiled into `ɵɵviewQuery(GridstackComponent, 5)`, which
        // names the predicate by its local name (ngtsc keeps the import too).
        let source = r#"
import { Component, ViewChild } from '@angular/core';
import { GridstackComponent, GridstackModule } from 'gridstack/dist/angular';

@Component({ selector: 'test', imports: [GridstackModule] })
class MyComponent {
    @ViewChild(GridstackComponent, { static: true })
    declare gridstack: GridstackComponent;
}
"#;
        let type_only = analyze_source(source);

        assert!(
            !type_only.contains("GridstackComponent"),
            "GridstackComponent is the query predicate the definition names"
        );

        // ViewChild is only used as decorator on a `declare` property, which compilation strips
        assert!(
            type_only.contains("ViewChild"),
            "ViewChild decorator on declare property should be elided"
        );

        // GridstackModule is used in Component imports array — must NOT be elided
        assert!(
            !type_only.contains("GridstackModule"),
            "GridstackModule should be preserved (value usage in imports array)"
        );
    }

    #[test]
    fn test_computed_property_key_in_type_annotation_preserved() {
        // Case 3: `[fromEmail]: Emailer[]` as computed property key in a type annotation.
        // Even though `[fromEmail]` appears in a type context, the computed property key
        // references the runtime value of `fromEmail`. Angular preserves it.
        let source = r#"
import { Component, Input } from '@angular/core';
import { fromEmail, RecipientType } from './email.interface';

@Component({ selector: 'test' })
class MyComponent {
    @Input() collapseEmailInfo: {
        [fromEmail]: string[];
        [RecipientType.To]: string[];
    };
}
"#;
        let type_only = analyze_source(source);

        // fromEmail should NOT be elided — it's a computed property key that references a value
        assert!(
            !type_only.contains("fromEmail"),
            "fromEmail used as computed property key in type annotation should be preserved"
        );

        // RecipientType is used via member access — should NOT be elided
        assert!(
            !type_only.contains("RecipientType"),
            "RecipientType used as computed property key in type annotation should be preserved"
        );
    }

    #[test]
    fn test_nested_type_literal_computed_key_preserved() {
        // Computed key inside a nested type literal: { nested: { [fromEmail]: string } }
        let source = r#"
import { Component, Input } from '@angular/core';
import { fromEmail } from './email.interface';

@Component({ selector: 'test' })
class MyComponent {
    @Input() config: {
        nested: {
            [fromEmail]: string;
        };
    };
}
"#;
        let type_only = analyze_source(source);

        assert!(
            !type_only.contains("fromEmail"),
            "fromEmail in nested type literal should be preserved"
        );
    }

    #[test]
    fn test_deeply_nested_type_literal_computed_key_preserved() {
        // Computed key three levels deep
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() data: {
        level1: {
            level2: {
                [myKey]: number;
            };
        };
    };
}
"#;
        let type_only = analyze_source(source);

        assert!(
            !type_only.contains("myKey"),
            "myKey in deeply nested type literal should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_nested_union_type_literal_preserved() {
        // Computed key inside a type literal nested within a union
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() data: {
        field: { [myKey]: string } | null;
    };
}
"#;
        let type_only = analyze_source(source);

        assert!(
            !type_only.contains("myKey"),
            "myKey in type literal nested within union should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_array_type_preserved() {
        // Review claim: TSArrayType `{ [key]: string }[]` is not handled
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() items: { [myKey]: string }[];
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey in array element type literal should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_generic_type_arg_preserved() {
        // Review claim: TSTypeReference `Array<{ [key]: string }>` is not handled
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() items: Array<{ [myKey]: string }>;
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey in generic type argument type literal should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_tuple_type_preserved() {
        // Review claim: TSTupleType is not handled
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() pair: [string, { [myKey]: number }];
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey in tuple element type literal should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_optional_tuple_element_preserved() {
        // TSOptionalType (`[number?]`) in a tuple previously caused a panic via
        // `to_ts_type()` (which called `unwrap()` on None). The fix uses
        // `as_ts_type()` and handles the three named TSTupleElement variants.
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() pair: [string?, { [myKey]: number }?];
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey in optional tuple element type literal should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_rest_tuple_element_preserved() {
        // TSRestType (`[...T[]]`) in a tuple previously caused a panic via
        // `to_ts_type()`. The fix explicitly unwraps `TSRestType`.
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() pair: [string, ...{ [myKey]: number }[]];
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey inside rest tuple element type should be preserved"
        );
    }

    #[test]
    fn test_computed_key_in_named_tuple_member_preserved() {
        // TSNamedTupleMember (`[name: T]`) in a tuple previously caused a panic
        // via `to_ts_type()`. The fix explicitly unwraps `TSNamedTupleMember`.
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() pair: [label: string, item: { [myKey]: number }];
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey in named tuple member type literal should be preserved"
        );
    }

    #[test]
    fn test_tuple_with_mixed_element_kinds_no_panic() {
        // Regression: a tuple with optional, rest, and named members together
        // must not panic (TSOptionalType/TSRestType previously hit the unwrap in to_ts_type).
        let source = r#"
import { Component, Input } from '@angular/core';

@Component({ selector: 'test' })
class MyComponent {
    @Input() data: [label: string, value?: number, ...rest: string[]];
}
"#;
        let type_only = analyze_source(source);
        assert!(!type_only.contains("Component"), "Component should be preserved (decorator)");
    }

    #[test]
    fn test_computed_key_in_parenthesized_type_preserved() {
        // Review claim: TSParenthesizedType is not handled
        let source = r#"
import { Component, Input } from '@angular/core';
import { myKey } from './keys';

@Component({ selector: 'test' })
class MyComponent {
    @Input() data: ({ [myKey]: string });
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("myKey"),
            "myKey in parenthesized type literal should be preserved"
        );
    }

    // =========================================================================
    // Regression tests for ClickUp import elision mismatches
    // =========================================================================

    #[test]
    fn test_namespace_import_type_only_should_be_elided() {
        // Reproduces: bookmark.component.ts
        // `import * as moment from 'moment'` where `moment` is only used as
        // `moment.Moment` in type annotations should be elided.
        let source = r#"
import * as moment from 'moment';
import { Component } from '@angular/core';

@Component({ selector: 'app-bookmark' })
class BookmarkComponent {
    dueDate: moment.Moment = null;
}
"#;
        let type_only = analyze_source(source);
        // `moment` is only referenced in type position (moment.Moment)
        // so it should be marked for elision
        assert!(
            type_only.contains("moment"),
            "Namespace import `moment` used only in type annotation `moment.Moment` should be elided"
        );
    }

    #[test]
    fn test_namespace_import_with_value_usage_preserved() {
        // Namespace import that IS used at runtime should be preserved
        let source = r#"
import * as moment from 'moment';
import { Component } from '@angular/core';

@Component({ selector: 'app-test' })
class TestComponent {
    now = moment();
}
"#;
        let type_only = analyze_source(source);
        assert!(
            !type_only.contains("moment"),
            "Namespace import `moment` used in value expression `moment()` should be preserved"
        );
    }

    #[test]
    fn test_forward_ref_arrow_function_preserves_value_use() {
        // Reproduces: tags.component.ts
        // `forwardRef(() => TagPickerComponent)` in @Component imports array
        // uses TagPickerComponent as a value inside an arrow function.
        // The arrow function body MUST be traversed to find value uses.
        let source = r#"
import { Component, forwardRef, Inject, Optional, SkipSelf } from '@angular/core';
import { TagPickerComponent } from './tag-picker/tag-picker.component';

@Component({
    selector: 'app-tags',
    imports: [forwardRef(() => TagPickerComponent)]
})
class TagsComponent {
    constructor(
        @Optional()
        @SkipSelf()
        @Inject(TagPickerComponent)
        readonly tagPickerComponent: TagPickerComponent,
    ) {}
}
"#;
        let type_only = analyze_source(source);
        // TagPickerComponent is used as a value in forwardRef(() => TagPickerComponent)
        // and as @Inject(TagPickerComponent) argument. It must NOT be elided.
        assert!(
            !type_only.contains("TagPickerComponent"),
            "TagPickerComponent used in forwardRef arrow function and @Inject should be preserved"
        );
    }

    #[test]
    fn test_empty_import_should_be_elided() {
        // Reproduces: users-table.component.ts
        // `import {} from '@cu/teams-pulse/types'` is an empty import that
        // should be completely removed (no specifiers to keep).
        let source = r#"
import { Component } from '@angular/core';
import {} from '@cu/teams-pulse/types';

@Component({ selector: 'app-users-table' })
class UsersTableComponent {}
"#;
        let filtered = filter_source(source);

        // The empty import should be removed entirely
        assert!(
            !filtered.contains("@cu/teams-pulse/types"),
            "Empty import `import {{}} from '@cu/teams-pulse/types'` should be removed.\nFiltered:\n{}",
            filtered
        );
    }

    #[test]
    fn test_export_type_with_import_both_removed() {
        let source = r#"
import { Config } from './config';
export type { Config };
"#;
        let filtered = filter_source(source);
        assert!(
            !filtered.contains("Config"),
            "Both import and export type should be removed.\nFiltered:\n{}",
            filtered
        );
        assert!(
            !filtered.contains("export"),
            "export type declaration should be removed.\nFiltered:\n{}",
            filtered
        );
    }

    #[test]
    fn test_export_type_multiple_specifiers_removed() {
        let source = r#"
import { Foo, Bar } from './types';
export type { Foo, Bar };
"#;
        let filtered = filter_source(source);
        assert!(!filtered.contains("Foo"), "Foo should be removed.\nFiltered:\n{}", filtered);
        assert!(!filtered.contains("Bar"), "Bar should be removed.\nFiltered:\n{}", filtered);
    }

    #[test]
    fn test_export_mixed_type_and_value_specifiers() {
        let source = r#"
import { Component } from '@angular/core';
import { Foo, Bar } from './types';
export { type Foo, Bar };

@Component({ selector: 'test' })
class TestComponent {
    value = Bar;
}
"#;
        let filtered = filter_source(source);
        // type Foo should be removed, Bar should remain
        assert!(
            !filtered.contains("Foo"),
            "type-only Foo should be removed from export.\nFiltered:\n{}",
            filtered
        );
        assert!(
            filtered.contains("export { Bar }"),
            "Value export Bar should remain.\nFiltered:\n{}",
            filtered
        );
    }

    #[test]
    fn test_export_type_with_source_removed() {
        let source = r#"
import { Component } from '@angular/core';
export type { Config } from './config';

@Component({ selector: 'test' })
class TestComponent {}
"#;
        let filtered = filter_source(source);
        assert!(
            !filtered.contains("Config"),
            "export type with source should be removed.\nFiltered:\n{}",
            filtered
        );
    }

    #[test]
    fn test_value_export_not_affected() {
        let source = r#"
import { Component } from '@angular/core';
import { helper } from './utils';
export { helper };

@Component({ selector: 'test' })
class TestComponent {
    value = helper();
}
"#;
        let filtered = filter_source(source);
        assert!(
            filtered.contains("export { helper }"),
            "Value export should not be affected.\nFiltered:\n{}",
            filtered
        );
    }

    /// A module specifier with a lone surrogate has no `&str` form: an import
    /// from it keeps all its bindings (generated code can't import them
    /// again), and a rebuilt re-export keeps the specifier as written.
    #[test]
    fn test_lone_surrogate_module_specifier_is_kept_as_written() {
        let source = r#"import { Used, OnlyType } from '\uD800';
export { type T, V } from '\uDC00';
let x: OnlyType = Used;
"#;
        let filtered = filter_source(source);
        assert_eq!(
            filtered,
            r#"import { Used, OnlyType } from '\uD800';
export { V } from '\uDC00';
let x: OnlyType = Used;
"#
        );
    }
}

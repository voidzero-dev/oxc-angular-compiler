//! ngtsc's source-file validation rules (`ngtsc/validation/`).
//!
//! Unlike the per-class checks in [`crate::directive`], these run on every
//! call expression in the file, wherever it appears.

use oxc_ast::ast::{
    CallExpression, ClassType, Expression, IdentifierReference, ImportDeclarationSpecifier,
    Program, Statement,
};
use oxc_ast::ast_kind::AstKind;
use oxc_ast_visit::Visit;
use oxc_diagnostics::OxcDiagnostic;
use oxc_semantic::{AstNode, Semantic, SemanticBuilder};

use crate::directive::{
    INPUT_API, InitializerApi, MODEL_API, OUTPUT_API, OUTPUT_FROM_OBSERVABLE_API, QUERY_APIS,
    StringConsts, decorator_written_name, initializer_api, is_angular_core_decorator,
};

/// The APIs ngtsc's `InitializerApiUsageRule` checks (`APIS_TO_CHECK`).
const APIS: [InitializerApi; 8] = [
    INPUT_API,
    MODEL_API,
    OUTPUT_API,
    OUTPUT_FROM_OBSERVABLE_API,
    QUERY_APIS[0],
    QUERY_APIS[1],
    QUERY_APIS[2],
    QUERY_APIS[3],
];

/// ngtsc's `shouldCheck`: the file imports one of the initializer functions
/// by name (under any alias) from its module, or the module as a namespace.
fn should_check(program: &Program<'_>) -> bool {
    program.body.iter().any(|stmt| {
        let Statement::ImportDeclaration(import) = stmt else { return false };
        let module = import.source.value.as_str();
        if module != "@angular/core" && module != "@angular/core/rxjs-interop" {
            return false;
        }
        import.specifiers.as_ref().is_some_and(|specifiers| {
            specifiers.iter().any(|specifier| match specifier {
                ImportDeclarationSpecifier::ImportSpecifier(specifier) => {
                    APIS.iter().any(|&(name, owning_module)| {
                        specifier.imported.name().as_str() == name && module == owning_module
                    })
                }
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(_) => true,
                ImportDeclarationSpecifier::ImportDefaultSpecifier(_) => false,
            })
        })
    })
}

/// The module that imports `id`'s declaration — the `ImportDeclaration`
/// containing it, like upstream's `getContainingImportDeclaration`.
fn importing_module<'a>(declaration: &AstNode<'a>, semantic: &Semantic<'a>) -> Option<&'a str> {
    let AstKind::ImportDeclaration(import) = semantic.nodes().parent_kind(declaration.id()) else {
        return None;
    };
    Some(import.source.value.as_str())
}

/// The declaration `id`'s reference resolves to when it's the symbol's only
/// declaration: ngtsc's `symbol.declarations.length === 1` —
/// `getSymbolAtLocation` on a shadowed identifier lands on the parameter or
/// local, not the import.
fn sole_declaration<'s, 'a>(
    id: &IdentifierReference<'a>,
    semantic: &'s Semantic<'a>,
) -> Option<&'s AstNode<'a>> {
    let reference_id = id.reference_id.get()?;
    let symbol_id = semantic.scoping().get_reference(reference_id).symbol_id()?;
    (semantic.scoping().symbol_declarations(symbol_id).count() == 1)
        .then(|| semantic.symbol_declaration(symbol_id))
}

/// ngtsc's `getDirectImportOfIdentifier`: the `(module, exported name)` `id`
/// imports, when its symbol's only declaration is an import specifier —
/// `import {f}` gives the imported name, `import f` and `import * as f`
/// fall back to the local one (`getExportedName`). Anything else resolves
/// to `None`.
fn direct_import_of<'a>(
    id: &IdentifierReference<'a>,
    semantic: &Semantic<'a>,
) -> Option<(&'a str, &'a str)> {
    let declaration = sole_declaration(id, semantic)?;
    let name = match declaration.kind() {
        AstKind::ImportSpecifier(specifier) => specifier.imported.name().as_str(),
        AstKind::ImportDefaultSpecifier(_) | AstKind::ImportNamespaceSpecifier(_) => {
            id.name.as_str()
        }
        _ => return None,
    };
    Some((importing_module(declaration, semantic)?, name))
}

/// ngtsc's `getImportOfNamespacedIdentifier`: the module `id`'s symbol is a
/// namespace import of — `ns` in `ns.f`, where the caller checks `f`.
fn namespace_import_of<'a>(
    id: &IdentifierReference<'a>,
    semantic: &Semantic<'a>,
) -> Option<&'a str> {
    let declaration = sole_declaration(id, semantic)?;
    if !matches!(declaration.kind(), AstKind::ImportNamespaceSpecifier(_)) {
        return None;
    }
    importing_module(declaration, semantic)
}

/// ngtsc's last `tryParseInitializerApi` step, `reflector.getImportOfIdentifier`
/// on the API's identifier — "using the type checker ... accounts for things
/// like shadowed variables": `f` (or `f` in `f.required`) only when `id`
/// resolves to an import of `api`'s name from its module; `ns.f` (or `ns.f`
/// in `ns.f.required`) only when `ns` resolves to a namespace import of the
/// module. Any other binding — a parameter, a local, an unrelated import —
/// means the call isn't one of the API's.
fn is_api_reference(callee: &Expression<'_>, api: InitializerApi, semantic: &Semantic<'_>) -> bool {
    // `f` — its binding must be the import.
    let named = |expr: &Expression<'_>| {
        let Expression::Identifier(id) = expr else { return false };
        direct_import_of(id, semantic)
            .is_some_and(|(module, name)| module == api.1 && name == api.0)
    };
    // `ns.f` — `ns` must be a namespace import of the module.
    let namespaced = |expr: &Expression<'_>| {
        let Expression::StaticMemberExpression(member) = expr else { return false };
        let Expression::Identifier(ns) = &member.object else { return false };
        member.property.name == api.0 && namespace_import_of(ns, semantic) == Some(api.1)
    };
    named(callee)
        || namespaced(callee)
        || match callee {
            // `f.required` / `ns.f.required` — the API reference is the object.
            Expression::StaticMemberExpression(member) if member.property.name == "required" => {
                named(&member.object) || namespaced(&member.object)
            }
            _ => false,
        }
}

/// `InitializerApiUsageRule::checkNode` over every call expression.
struct InitializerApiUsage<'c, 'a> {
    consts: &'c StringConsts<'a>,
    /// The semantic model, for resolving callee identifiers to their binding
    /// like `getSymbolAtLocation` does upstream.
    semantic: &'c Semantic<'a>,
    /// The `AstKind`s enclosing the node being entered, innermost last.
    ancestors: std::vec::Vec<AstKind<'a>>,
    diagnostics: std::vec::Vec<OxcDiagnostic>,
}

impl<'a> InitializerApiUsage<'_, 'a> {
    fn check_call(&mut self, call: &'a CallExpression<'a>) {
        // The direct parent of the call. If it wraps the call in parentheses
        // or an `as` cast, upstream's `node` stops being a call expression
        // and the rule reports nothing — `x = (input(0))` and
        // `x = input(0) as any` never error. `!`/`satisfies` don't count.
        let Some(parent) = self.ancestors.last() else { return };
        if matches!(parent, AstKind::ParenthesizedExpression(_) | AstKind::TSAsExpression(_)) {
            return;
        }
        let Some((api, is_required)) = initializer_api(&call.callee, Some(self.consts), &APIS)
        else {
            return;
        };
        // The file-level import map is only the cheap pre-filter upstream's
        // `ImportedSymbolsTracker` plays; a shadowed name still passes it.
        if !is_api_reference(&call.callee, api, self.semantic) {
            return;
        }
        let function_name = format!("{}{}", api.0, if is_required { ".required" } else { "" });

        // The initializer of a class property (or auto-accessor) is the one
        // supported position, and only on a `@Component` / `@Directive` class
        // declaration. `closestClass` climbs past class expressions upstream,
        // to the nearest declaration.
        let prop = match parent {
            AstKind::PropertyDefinition(prop) => prop.value.as_ref(),
            AstKind::AccessorProperty(accessor) => accessor.value.as_ref(),
            _ => None,
        };
        if let Some(prop) = prop
            && matches!(prop, Expression::CallExpression(c) if std::ptr::eq(&raw const **c, call))
        {
            let class = self.ancestors.iter().rev().find_map(|kind| match kind {
                AstKind::Class(class) if class.r#type == ClassType::ClassDeclaration => {
                    Some(*class)
                }
                _ => None,
            });
            if let Some(class) = class {
                let is_component_or_directive = class.decorators.iter().any(|d| {
                    matches!(decorator_written_name(d), "Component" | "Directive")
                        && is_angular_core_decorator(d, Some(self.consts))
                });
                if !is_component_or_directive {
                    self.diagnostics.push(
                        OxcDiagnostic::error(format!(
                            "Unsupported call to the {function_name} function. \
                             This function can only be used as the initializer \
                             of a property on a @Component or @Directive class."
                        ))
                        .with_label(call.span),
                    );
                }
                return;
            }
        }
        self.diagnostics.push(
            OxcDiagnostic::error(format!(
                "Unsupported call to the {function_name} function. \
                 This function can only be called in the initializer of a class member."
            ))
            .with_label(call.span),
        );
    }
}

impl<'a> Visit<'a> for InitializerApiUsage<'_, 'a> {
    fn enter_node(&mut self, kind: AstKind<'a>) {
        if let AstKind::CallExpression(call) = kind {
            self.check_call(call);
        }
        self.ancestors.push(kind);
    }

    fn leave_node(&mut self, _kind: AstKind<'a>) {
        self.ancestors.pop();
    }
}

/// ngtsc's `InitializerApiUsageRule` (UNSUPPORTED_INITIALIZER_API_USAGE): a
/// diagnostic for every call to an initializer API outside the initializer of
/// a property on a `@Component` / `@Directive` class.
pub fn initializer_api_usage_errors<'a>(
    program: &'a Program<'a>,
    consts: &StringConsts<'a>,
) -> std::vec::Vec<OxcDiagnostic> {
    if !should_check(program) {
        return std::vec::Vec::new();
    }
    // Built here so `IdentifierReference.reference_id`s are populated for the
    // visitor's binding resolution — upstream's `checker.getSymbolAtLocation`.
    // `with_build_nodes` keeps the node store for the declaration kinds and
    // their `ImportDeclaration` parents; resolution doesn't need it.
    let semantic_ret = SemanticBuilder::new().with_build_nodes(true).build(program);
    let mut rule = InitializerApiUsage {
        consts,
        semantic: &semantic_ret.semantic,
        ancestors: std::vec::Vec::new(),
        diagnostics: std::vec::Vec::new(),
    };
    rule.visit_program(program);
    rule.diagnostics
}

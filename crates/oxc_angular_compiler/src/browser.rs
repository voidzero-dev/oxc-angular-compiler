//! TypeScript stripped as `tsc` does it under Angular's settings, and `async` lowered for zone.js.
//!
//! This is what a bundler without per-feature lowering needs to build an Angular app that runs zone.js: zone.js
//! cannot see native `await`, so the Angular CLI turns `async` functions into generators (esbuild's `supported`
//! option) and leaves everything else native.

use std::path::{Path, PathBuf};

use oxc_allocator::{Allocator, CloneIn, TakeIn, Vec as ArenaVec};
use oxc_ast::ast::{
    AssignmentTarget, ClassBody, ClassElement, Expression, IdentifierName, PropertyKey, Statement,
};
use oxc_ast::builder::AstBuilder;
use oxc_ast_visit::{VisitMut, walk_mut};
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_diagnostics::{OxcDiagnostic, Severity};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::{SPAN, SourceType};
use oxc_syntax::operator::AssignmentOperator;
use oxc_transformer::{
    CompilerAssumptions, DecoratorOptions, ES2017Options, ES2018Options, ES2026Options, EnvOptions,
    HelperLoaderMode, HelperLoaderOptions, TransformOptions, Transformer, TypeScriptOptions,
};

/// Options for [`transform_for_browser`].
#[derive(Debug, Clone, Default)]
pub struct BrowserTransformOptions {
    /// Generate a source map (default: false).
    pub sourcemap: bool,
    /// `async` functions and generators, and `for await`, as generators (default: false; the napi binding
    /// defaults to true). Helpers are imported from `@oxc-project/runtime`.
    pub lower_async: bool,
}

/// Result of [`transform_for_browser`].
#[derive(Debug, Default)]
pub struct BrowserTransformResult {
    /// The transformed code; empty when there are errors.
    pub code: String,
    /// Source map (if the `sourcemap` option was enabled).
    pub map: Option<String>,
    /// Parse and transform errors.
    pub errors: Vec<OxcDiagnostic>,
}

/// Strips TypeScript and lowers `async`, as the Angular CLI does for an app that runs zone.js.
///
/// TypeScript files get `experimentalDecorators` and `useDefineForClassFields: false` semantics, as `tsc` emits them
/// for target ES2022: fields are assigned in the constructor after parameter properties, statics in static blocks,
/// and `#private` members stay native. JavaScript keeps its class fields. `using` declarations are lowered in
/// TypeScript files (as `tsc` does) and wherever `async` is lowered (`await using` cannot live in a generator).
pub fn transform_for_browser(
    allocator: &Allocator,
    code: &str,
    filename: &str,
    options: &BrowserTransformOptions,
) -> BrowserTransformResult {
    // Bundler ids can carry a query (`a.ts?v=1`); the extension comes before it.
    let path = filename.split_once('?').map_or(filename, |(path, _)| path);
    let mut source_type = SourceType::from_path(path).unwrap_or_else(|_| SourceType::mjs());
    let typescript = source_type.is_typescript();
    // Angular compiles TypeScript as ES modules, so a file without `import`/`export` still gets `import`ed helpers.
    if typescript && source_type.is_unambiguous() {
        source_type = source_type.with_module(true);
    }
    let parsed = Parser::new(allocator, code, source_type).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return BrowserTransformResult {
            errors: only_errors(parsed.diagnostics.into_vec()),
            ..Default::default()
        };
    }
    let mut program = parsed.program;
    if typescript {
        let mut split = SplitDecoratedStaticFields { allocator, errors: Vec::new() };
        split.visit_program(&mut program);
        if !split.errors.is_empty() {
            return BrowserTransformResult { errors: split.errors, ..Default::default() };
        }
    }

    let mut env = EnvOptions::default();
    if options.lower_async {
        env.es2017 = ES2017Options { async_to_generator: true };
        env.es2018 = ES2018Options { async_generator_functions: true, ..env.es2018 };
    }
    if options.lower_async || typescript {
        env.es2026 = ES2026Options { explicit_resource_management: true };
    }
    // No class-properties plugin: with `set_public_class_fields`, the TypeScript transform alone moves fields with an
    // initializer into the constructor and statics into static blocks, and keeps `#private` members, as `tsc` does.
    let transform_options = TransformOptions {
        cwd: PathBuf::new(),
        typescript: TypeScriptOptions {
            remove_class_fields_without_initializer: true,
            ..Default::default()
        },
        assumptions: CompilerAssumptions { set_public_class_fields: true, ..Default::default() },
        decorator: DecoratorOptions { legacy: typescript, ..Default::default() },
        env,
        helper_loader: HelperLoaderOptions {
            mode: HelperLoaderMode::Runtime,
            ..Default::default()
        },
        ..Default::default()
    };
    let scoping = SemanticBuilder::new()
        .with_excess_capacity(2.0)
        .with_enum_eval(true)
        .build(&program)
        .semantic
        .into_scoping();
    let transformed = Transformer::new(allocator, Path::new(path), &transform_options)
        .build_with_scoping(scoping, &mut program);
    if transformed.diagnostics.has_errors() {
        return BrowserTransformResult {
            errors: only_errors(transformed.diagnostics.into_vec()),
            ..Default::default()
        };
    }

    let codegen = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: options.sourcemap.then(|| PathBuf::from(path)),
            ..CodegenOptions::default()
        })
        .with_source_text(code)
        .build(&program);
    BrowserTransformResult {
        code: codegen.code,
        map: codegen.map.map(|m| m.to_json_string()),
        errors: Vec::new(),
    }
}

/// Rewrites `@D static x = 1;` as `@D static x; static { this.x = 1; }`, which is what `tsc` emits. oxc's TypeScript
/// transform turns an initialized static field into a static block itself, before the legacy decorator pass sees the
/// field, and the decorator is lost; a field without an initializer keeps it.
struct SplitDecoratedStaticFields<'a> {
    allocator: &'a Allocator,
    errors: Vec<OxcDiagnostic>,
}

impl<'a> VisitMut<'a> for SplitDecoratedStaticFields<'a> {
    fn visit_class_body(&mut self, body: &mut ClassBody<'a>) {
        walk_mut::walk_class_body(self, body);
        let splits = |e: &ClassElement<'a>| {
            matches!(e, ClassElement::PropertyDefinition(p)
                if p.r#static && !p.declare && !p.decorators.is_empty() && p.value.is_some())
        };
        if !body.body.iter().any(splits) {
            return;
        }
        let allocator = self.allocator;
        let ast = AstBuilder::new(allocator);
        let mut elements = ArenaVec::with_capacity_in(body.body.len() + 1, &ast);
        for mut element in body.body.take_in(&ast) {
            if !splits(&element) {
                elements.push(element);
                continue;
            }
            let ClassElement::PropertyDefinition(prop) = &mut element else { unreachable!() };
            let this = Expression::new_this_expression(SPAN, &ast);
            let target = match &prop.key {
                PropertyKey::StaticIdentifier(id) if !prop.computed => {
                    let name = IdentifierName::new(id.span, id.name, &ast);
                    AssignmentTarget::new_static_member_expression(SPAN, this, name, false, &ast)
                }
                PropertyKey::StringLiteral(lit) if !prop.computed => {
                    let key = Expression::StringLiteral(lit.clone_in(allocator));
                    AssignmentTarget::new_computed_member_expression(SPAN, this, key, false, &ast)
                }
                PropertyKey::NumericLiteral(lit) if !prop.computed => {
                    let key = Expression::NumericLiteral(lit.clone_in(allocator));
                    AssignmentTarget::new_computed_member_expression(SPAN, this, key, false, &ast)
                }
                _ => {
                    self.errors.push(
                        OxcDiagnostic::error(
                            "A decorated static field with a computed key and an initializer is not supported",
                        )
                        .with_label(prop.span),
                    );
                    elements.push(element);
                    continue;
                }
            };
            let value = prop.value.take().unwrap();
            let assign = Expression::new_assignment_expression(
                SPAN,
                AssignmentOperator::Assign,
                target,
                value,
                &ast,
            );
            let block = ClassElement::new_static_block(
                prop.span,
                ArenaVec::from_value_in(
                    Statement::new_expression_statement(SPAN, assign, &ast),
                    &ast,
                ),
                &ast,
            );
            elements.push(element);
            elements.push(block);
        }
        body.body = elements;
    }
}

fn only_errors(diagnostics: Vec<OxcDiagnostic>) -> Vec<OxcDiagnostic> {
    diagnostics.into_iter().filter(|d| d.severity == Severity::Error).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(code: &str, filename: &str) -> String {
        let allocator = Allocator::default();
        let r = transform_for_browser(
            &allocator,
            code,
            filename,
            &BrowserTransformOptions { sourcemap: false, lower_async: true },
        );
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        r.code
    }

    #[test]
    fn lowers_async_and_for_await() {
        let out = run(
            "export async function f(xs) { for await (const x of xs) await x; }\nexport async function* g() { yield 1; }",
            "a.mjs",
        );
        assert!(!out.contains("async function"), "{out}");
        assert!(!out.contains("for await"), "{out}");
        assert!(out.contains("@oxc-project/runtime/helpers/asyncToGenerator"), "{out}");
    }

    #[test]
    fn typescript_fields_are_assigned_after_parameter_properties() {
        let out = run(
            "export class A { b = this.a.x; static s = 1; c: number; constructor(private a: { x: number }) {} }",
            "a.ts",
        );
        let a = out.find("this.a = a").expect(&out);
        let b = out.find("this.b = this.a.x").expect(&out);
        assert!(a < b, "{out}");
        assert!(out.contains("static {\n\t\tthis.s = 1;\n\t}"), "{out}");
        assert!(!out.contains("this.c"), "{out}");
    }

    #[test]
    fn typescript_static_blocks_keep_source_order() {
        let out =
            run("export class A { static s = 1; static { log(A.s); } static t = 2; }", "a.ts");
        let s = out.find("this.s = 1").expect(&out);
        let log = out.find("log(A.s)").expect(&out);
        let t = out.find("this.t = 2").expect(&out);
        assert!(s < log && log < t, "{out}");
    }

    #[test]
    fn typescript_keeps_private_members_native() {
        let out = run("export class A { #x = 1; b = this.#m(); #m() { return this.#x; } }", "a.ts");
        assert!(
            out.contains("#x;") && out.contains("this.#x = 1") && out.contains("#m() {"),
            "{out}"
        );
        assert!(!out.contains("@oxc-project/runtime"), "{out}");
    }

    #[test]
    fn typescript_enums_and_namespaces() {
        let out = run(
            "enum E { A, B = A + 2 } namespace N { export const a = E.B; } export { E, N };",
            "a.ts",
        );
        assert!(out.contains("E[E[\"B\"] = 2] = \"B\""), "{out}");
        assert!(out.contains("_N.a = E.B"), "{out}");
    }

    #[test]
    fn typescript_keeps_decorators_of_initialized_static_fields() {
        let out = run(
            "function D(t: any, k: any) {} export class A { @D static foo = 1; @D static 'a-b' = 2; @D static bar: any; }",
            "a.ts",
        );
        assert!(out.contains("static {\n\t\tthis.foo = 1;\n\t}"), "{out}");
        assert!(out.contains("this[\"a-b\"] = 2"), "{out}");
        for key in ["\"foo\"", "\"a-b\"", "\"bar\""] {
            assert!(out.contains(&format!("_decorate([D], A, {key}, void 0)")), "{key}: {out}");
        }
    }

    #[test]
    fn typescript_decorated_static_field_with_computed_key_is_an_error() {
        let allocator = Allocator::default();
        let r = transform_for_browser(
            &allocator,
            "function D(t: any, k: any) {} export class A { @D static [Symbol.iterator] = 1; }",
            "a.ts",
            &BrowserTransformOptions::default(),
        );
        assert!(r.code.is_empty());
        assert_eq!(r.errors.len(), 1, "{:?}", r.errors);
    }

    #[test]
    fn lowers_await_using_before_async() {
        let out = run("export async function f(r) { await using x = r; return 1; }", "a.mjs");
        assert!(!out.contains("using x"), "{out}");
        assert!(!out.contains("async function"), "{out}");
    }

    #[test]
    fn typescript_without_import_or_export_imports_helpers() {
        let out = run(
            "addEventListener('message', async ({ data }) => postMessage(await data));",
            "worker.ts",
        );
        assert!(out.contains("import _asyncToGenerator from"), "{out}");
        assert!(!out.contains("require("), "{out}");
    }

    #[test]
    fn filename_query_is_ignored() {
        let out = run("export class A { x: number = 1; }", "a.ts?v=1");
        assert!(out.contains("this.x = 1"), "{out}");
    }

    #[test]
    fn javascript_keeps_class_fields() {
        let out = run("export class A { b = 1; static s = 2; #p = 3; }", "a.mjs");
        assert!(
            out.contains("b = 1;") && out.contains("static s = 2") && out.contains("#p = 3"),
            "{out}"
        );
    }
}

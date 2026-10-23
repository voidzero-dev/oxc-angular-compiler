//! What a bundler without per-feature lowering needs for an Angular app that runs zone.js: TypeScript stripped as
//! `tsc` does it under Angular's settings, and `async`/`await` turned into generators, because zone.js cannot see
//! native `await` (the Angular CLI does the same through esbuild's `supported` option).

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_codegen::{Codegen, CodegenOptions};
use oxc_parser::Parser;
use oxc_semantic::SemanticBuilder;
use oxc_span::SourceType;
use oxc_transformer::{
    ClassPropertiesOptions, CompilerAssumptions, DecoratorOptions, ES2017Options, ES2018Options,
    ES2022Options, EnvOptions, HelperLoaderMode, HelperLoaderOptions, TransformOptions,
    Transformer, TypeScriptOptions,
};

#[derive(Debug, Clone, Default)]
pub struct BrowserTransformOptions {
    pub sourcemap: bool,
    /// `async` functions and generators, and `for await`, as generators (helpers from `@oxc-project/runtime`)
    pub lower_async: bool,
}

#[derive(Debug, Default)]
pub struct BrowserTransformResult {
    pub code: String,
    pub map: Option<String>,
    pub errors: Vec<String>,
}

/// TypeScript files get `experimentalDecorators` and `useDefineForClassFields: false` semantics (fields assigned in
/// the constructor, after parameter properties; statics assigned after the class), as Angular's tsconfig has them.
/// JavaScript keeps its class fields.
pub fn transform_for_browser(
    allocator: &Allocator,
    code: &str,
    filename: &str,
    options: &BrowserTransformOptions,
) -> BrowserTransformResult {
    let source_type = SourceType::from_path(filename).unwrap_or_else(|_| SourceType::mjs());
    let parsed = Parser::new(allocator, code, source_type).parse();
    if parsed.fatal_error || parsed.diagnostics.has_errors() {
        return BrowserTransformResult {
            errors: parsed.diagnostics.errors().map(|e| e.to_string()).collect(),
            ..Default::default()
        };
    }
    let mut program = parsed.program;
    let typescript = source_type.is_typescript();

    let mut env = EnvOptions::default();
    if options.lower_async {
        env.es2017 = ES2017Options { async_to_generator: true };
        env.es2018 = ES2018Options { async_generator_functions: true, ..env.es2018 };
    }
    if typescript {
        env.es2022 = ES2022Options {
            class_properties: Some(ClassPropertiesOptions { loose: true }),
            ..env.es2022
        };
    }
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
    let scoping =
        SemanticBuilder::new().with_excess_capacity(2.0).build(&program).semantic.into_scoping();
    let transformed = Transformer::new(allocator, Path::new(filename), &transform_options)
        .build_with_scoping(scoping, &mut program);
    if transformed.diagnostics.has_errors() {
        return BrowserTransformResult {
            errors: transformed.diagnostics.errors().map(|e| e.to_string()).collect(),
            ..Default::default()
        };
    }

    let codegen = Codegen::new()
        .with_options(CodegenOptions {
            source_map_path: options.sourcemap.then(|| PathBuf::from(filename)),
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
        assert!(out.contains("A.s = 1"), "{out}");
        assert!(!out.contains("this.c"), "{out}");
    }

    #[test]
    fn javascript_keeps_class_fields() {
        let out = run("export class A { b = 1; static s = 2; }", "a.mjs");
        assert!(out.contains("b = 1;") && out.contains("static s = 2"), "{out}");
    }
}

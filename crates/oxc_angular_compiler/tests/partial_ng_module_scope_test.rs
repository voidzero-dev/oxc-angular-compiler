//! NgModule scopes in partial compilation (#655): a component an NgModule
//! declares gets its compilation scope from `ɵɵsetComponentScope` in the
//! module's file, and a standalone component's `dependencies` list what the
//! NgModules it imports export, when cross-file analysis
//! (`TransformOptions::resolve_imported_values`) reads them.

#![cfg(feature = "cross_file_elision")]

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    AngularVersion, CompilationMode, TransformOptions, TransformResult, transform_angular_file,
};
use tempfile::TempDir;

fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn partial() -> TransformOptions {
    TransformOptions {
        compilation_mode: CompilationMode::Partial,
        resolve_imported_values: true,
        ..Default::default()
    }
}

fn compile(dir: &TempDir, name: &str, source: &str, options: &TransformOptions) -> TransformResult {
    let path = write(dir, name, source);
    let allocator = Allocator::default();
    let result =
        transform_angular_file(&allocator, path.to_str().unwrap(), source, Some(options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    result
}

fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// `src/dep.ts`: a directive and a pipe its NgModule declares and exports.
fn dep_module(dir: &TempDir) {
    write(
        dir,
        "src/dep.ts",
        r"
import { Directive, NgModule, Pipe } from '@angular/core';
@Directive({ selector: '[d]', standalone: false }) export class D {}
@Pipe({ name: 'p', standalone: false }) export class P { transform(v: unknown) { return v; } }
@NgModule({ declarations: [D, P], exports: [D, P] }) export class DepModule {}
",
    );
}

/// `node_modules/@lib/ui`: a package whose declaration files are laid out
/// like Angular's own: an entry re-exporting a chunk, an alias, a module
/// exporting another module, and `forRoot()`.
fn ui_package(dir: &TempDir) {
    write(
        dir,
        "node_modules/@lib/ui/package.json",
        r#"{ "name": "@lib/ui", "exports": { ".": { "types": "./types/ui.d.ts", "default": "./fesm/ui.mjs" } } }"#,
    );
    write(dir, "node_modules/@lib/ui/fesm/ui.mjs", "export {};\n");
    write(
        dir,
        "node_modules/@lib/ui/types/ui.d.ts",
        "export { UiDir as UiAlias, UiDir, UiPipe, UiModule } from './_chunk.js';\n",
    );
    write(
        dir,
        "node_modules/@lib/ui/types/_chunk.d.ts",
        r#"
import * as i0 from '@angular/core';
import * as i1 from './_inner.js';
declare class UiDir {
    static ɵdir: i0.ɵɵDirectiveDeclaration<UiDir, "[ui]", never, {}, {}, never, never, true, never>;
}
declare class UiPipe {
    static ɵpipe: i0.ɵɵPipeDeclaration<UiPipe, "ui", true>;
}
declare class UiModule {
    static forRoot(): i0.ModuleWithProviders<UiModule>;
    static ɵmod: i0.ɵɵNgModuleDeclaration<UiModule, never, [typeof UiDir, typeof UiPipe], [typeof UiDir, typeof UiPipe, typeof i1.InnerModule]>;
}
export { UiDir, UiPipe, UiModule };
"#,
    );
    write(
        dir,
        "node_modules/@lib/ui/types/_inner.d.ts",
        r#"
import * as i0 from '@angular/core';
export declare class InnerDir {
    static ɵdir: i0.ɵɵDirectiveDeclaration<InnerDir, "[inner]", never, {}, {}, never, never, false, never>;
}
export declare class InnerModule {
    static ɵmod: i0.ɵɵNgModuleDeclaration<InnerModule, [typeof InnerDir], never, [typeof InnerDir]>;
}
"#,
    );
}

#[test]
fn module_sets_the_scope_of_the_component_it_declares() {
    let dir = TempDir::new().unwrap();
    dep_module(&dir);
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, NgModule } from '@angular/core';
import { DepModule } from './dep.js';
@Component({ selector: 'app', template: '<div d>{{ 1 | p }}</div>', standalone: false })
export class App {}
@NgModule({ declarations: [App], imports: [DepModule] })
export class AppModule {}
",
        &partial(),
    );
    let code = compact(&result.code);
    // The file's own `./dep.js` import names the module the classes come from.
    assert!(code.contains("import*asi1from'./dep.js';"), "{}", result.code);
    assert!(
        code.contains("i0.ɵɵsetComponentScope(App,()=>[i1.D,App],()=>[i1.P]);"),
        "{}",
        result.code
    );
    // At the end of the file, once every class in it exists.
    assert!(code.ends_with("()=>[i1.P]);"), "{}", result.code);
    // The files read are watched.
    assert!(
        result.dependencies.iter().any(|file| file.ends_with("dep.ts")),
        "{:?}",
        result.dependencies
    );
}

#[test]
fn declaration_file_scopes_are_read_through_the_entry_point() {
    let dir = TempDir::new().unwrap();
    ui_package(&dir);
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, NgModule } from '@angular/core';
import { UiModule } from '@lib/ui';
@Component({ selector: 'app', template: '<div ui inner></div>', standalone: false })
export class App {}
@NgModule({ declarations: [App], imports: [UiModule.forRoot()] })
export class AppModule {}
",
        &partial(),
    );
    let code = compact(&result.code);
    // `UiDir` by its own name, not the `UiAlias` listed first. `InnerDir`,
    // which the entry point doesn't export, makes the scope unreadable.
    assert!(!code.contains("UiAlias"), "{}", result.code);
    assert!(!code.contains("ɵɵsetComponentScope"), "{}", result.code);
    // Package files aren't watched.
    assert!(
        !result.dependencies.iter().any(|file| file.contains("node_modules")),
        "{:?}",
        result.dependencies
    );
}

#[test]
fn declaration_file_scope_with_nested_module() {
    let dir = TempDir::new().unwrap();
    ui_package(&dir);
    // The entry point exports the nested module's directive too.
    write(
        &dir,
        "node_modules/@lib/ui/types/ui.d.ts",
        "export { UiDir as UiAlias, UiDir, UiPipe, UiModule } from './_chunk.js';\nexport * from './_inner.js';\n",
    );
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, NgModule } from '@angular/core';
import * as ui from '@lib/ui';
@Component({ selector: 'app', template: '<div ui inner></div>', standalone: false })
export class App {}
@NgModule({ declarations: [App], imports: [ui.UiModule.forRoot()] })
export class AppModule {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains(
            "i0.ɵɵsetComponentScope(App,()=>[i1.UiDir,i1.InnerDir,App],()=>[i1.UiPipe]);"
        ),
        "{}",
        result.code
    );
}

#[test]
fn lists_barrels_and_forward_refs_are_followed() {
    let dir = TempDir::new().unwrap();
    dep_module(&dir);
    write(&dir, "src/index.ts", "export * from './dep';\nexport { SHARED } from './shared';\n");
    write(
        &dir,
        "src/shared.ts",
        r"
import { DepModule } from './dep';
export const SHARED = [DepModule];
",
    );
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, NgModule, forwardRef } from '@angular/core';
import { SHARED } from './index';
@Component({ selector: 'a-cmp', template: '', standalone: false }) export class A {}
@Component({ selector: 'b-cmp', template: '', standalone: false }) export class B {}
const COMPONENTS = [A, forwardRef(() => B)];
@NgModule({ declarations: [...COMPONENTS], imports: [SHARED] })
export class AppModule {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains("i0.ɵɵsetComponentScope(A,()=>[i1.D,A,B],()=>[i1.P]);"),
        "{}",
        result.code
    );
    assert!(
        code.contains("i0.ɵɵsetComponentScope(B,()=>[i1.D,A,B],()=>[i1.P]);"),
        "{}",
        result.code
    );
    assert!(code.contains("import*asi1from'./dep';"), "{}", result.code);
}

#[test]
fn an_unreadable_scope_leaves_the_module_as_before() {
    let dir = TempDir::new().unwrap();
    for imports in ["[Missing]", "[makeModules()]", "[UnknownPkgModule]"] {
        let source = format!(
            r"
import {{ Component, NgModule }} from '@angular/core';
import {{ UnknownPkgModule }} from 'unknown-pkg';
declare const Missing: any;
declare function makeModules(): any[];
@Component({{ selector: 'app', template: '', standalone: false }}) export class App {{}}
@NgModule({{ declarations: [App], imports: {imports} }})
export class AppModule {{}}
"
        );
        let result = compile(&dir, "src/app.ts", &source, &partial());
        assert!(!result.code.contains("ɵɵsetComponentScope"), "{imports}: {}", result.code);
    }
}

#[test]
fn a_module_cycle_leaves_the_module_as_before() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "src/cycle.ts",
        r"
import { NgModule, forwardRef } from '@angular/core';
@NgModule({ exports: [forwardRef(() => BModule)] }) export class AModule {}
@NgModule({ exports: [AModule] }) export class BModule {}
",
    );
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, NgModule } from '@angular/core';
import { AModule } from './cycle';
@Component({ selector: 'app', template: '', standalone: false }) export class App {}
@NgModule({ declarations: [App], imports: [AModule] })
export class AppModule {}
",
        &partial(),
    );
    assert!(!result.code.contains("ɵɵsetComponentScope"), "{}", result.code);
}

#[test]
fn only_partial_compilation_with_imported_values_sets_scopes() {
    let dir = TempDir::new().unwrap();
    dep_module(&dir);
    let source = r"
import { Component, NgModule } from '@angular/core';
import { DepModule } from './dep';
@Component({ selector: 'app', template: '<div d></div>', standalone: false }) export class App {}
@NgModule({ declarations: [App], imports: [DepModule] })
export class AppModule {}
";
    let full = TransformOptions { compilation_mode: CompilationMode::Full, ..partial() };
    let no_values = TransformOptions { resolve_imported_values: false, ..partial() };
    for options in [full, no_values] {
        let result = compile(&dir, "src/app.ts", source, &options);
        assert!(!result.code.contains("ɵɵsetComponentScope"), "{}", result.code);
    }
}

#[test]
fn standalone_component_lists_what_an_imported_module_exports() {
    let dir = TempDir::new().unwrap();
    dep_module(&dir);
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component } from '@angular/core';
import { DepModule } from './dep';
@Component({ selector: 'app', template: '<div d>{{ 1 | p }}</div>', imports: [DepModule] })
export class App {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains(
            "dependencies:[{kind:\"ngmodule\",type:DepModule},{kind:\"directive\",type:i1.D,selector:\"[d]\"},{kind:\"pipe\",type:i1.P,name:\"p\"}]"
        ),
        "{}",
        result.code
    );
}

#[test]
fn standalone_component_keeps_an_unreadable_import_as_before() {
    let dir = TempDir::new().unwrap();
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component } from '@angular/core';
import { SomeModule } from 'unknown-pkg';
@Component({ selector: 'app', template: '', imports: [SomeModule] })
export class App {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains("dependencies:[{kind:\"directive\",type:SomeModule,selector:\"*\"}]"),
        "{}",
        result.code
    );
}

#[test]
fn declarations_carry_the_angular_version_compiled_for() {
    let dir = TempDir::new().unwrap();
    let source = r"
import { Component } from '@angular/core';
@Component({ selector: 'app', template: '@if (true) { x }' })
export class App {}
";
    let latest = compile(&dir, "src/app.ts", source, &partial());
    assert!(compact(&latest.code).contains("version:\"22.2.3\""), "{}", latest.code);
    assert!(!latest.code.contains("PLACEHOLDER"), "{}", latest.code);
    let v20 =
        TransformOptions { angular_version: Some(AngularVersion::new(20, 1, 4)), ..partial() };
    let result = compile(&dir, "src/app.ts", source, &v20);
    assert!(compact(&result.code).contains("version:\"20.1.4\""), "{}", result.code);
    assert!(!result.code.contains("22.2.3"), "{}", result.code);
    assert!(!result.code.contains("isStandalone"), "{}", result.code);
    // Angular's linker reads a missing `isStandalone` as `false` before v19.
    let v18 =
        TransformOptions { angular_version: Some(AngularVersion::new(18, 2, 0)), ..partial() };
    let standalone = source.replace("selector: 'app',", "selector: 'app', standalone: true,");
    let result = compile(&dir, "src/app.ts", &standalone, &v18);
    assert!(compact(&result.code).contains("isStandalone:true"), "{}", result.code);
}

#[test]
fn standalone_component_lists_each_class_once() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "src/shared.ts",
        r"
import { Directive, NgModule, Pipe } from '@angular/core';
@Directive({ selector: '[s]' }) export class S {}
@Pipe({ name: 'q' }) export class Q { transform(v: unknown) { return v; } }
@NgModule({ imports: [S, Q], exports: [S, Q] }) export class AModule {}
@NgModule({ imports: [S, Q], exports: [S, Q] }) export class BModule {}
",
    );
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component } from '@angular/core';
import { AModule, BModule, S } from './shared';
@Component({ selector: 'app', template: '<i s>{{ 1 | q }}</i>', imports: [S, AModule, BModule] })
export class App {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains(
            "dependencies:[{kind:\"directive\",type:S,selector:\"*\"},{kind:\"ngmodule\",type:AModule},{kind:\"pipe\",type:i1.Q,name:\"q\"},{kind:\"ngmodule\",type:BModule}]"
        ),
        "{}",
        result.code
    );
}

#[test]
fn a_component_in_an_import_cycle_with_its_module_gets_no_scope() {
    let dir = TempDir::new().unwrap();
    dep_module(&dir);
    // `cycle.ts` reaches the module file through `token.ts`, so it may run
    // after it: the module file can't read `Cycle` when it runs.
    write(&dir, "src/token.ts", "export { TOKEN } from './app.module';\n");
    write(
        &dir,
        "src/cycle.ts",
        r"
import { Component } from '@angular/core';
import { TOKEN } from './token';
@Component({ selector: 'cycle', template: '{{ token }}', standalone: false })
export class Cycle { token = TOKEN; }
",
    );
    write(
        &dir,
        "src/plain.ts",
        r"
import { Component } from '@angular/core';
@Component({ selector: 'plain', template: '', standalone: false }) export class Plain {}
",
    );
    let result = compile(
        &dir,
        "src/app.module.ts",
        r"
import { NgModule } from '@angular/core';
import { DepModule } from './dep';
import { Cycle } from './cycle';
import { Plain } from './plain';
export const TOKEN = 'token';
@NgModule({ declarations: [Cycle, Plain], imports: [DepModule] })
export class AppModule {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert_eq!(code.matches("ɵɵsetComponentScope").count(), 1, "{}", result.code);
    assert!(
        code.contains("ɵɵsetComponentScope(i1.Plain,()=>[i2.D,i3.Cycle,i1.Plain],()=>[i2.P]);"),
        "{}",
        result.code
    );
}

#[test]
fn export_star_cycles_end() {
    use std::fmt::Write as _;
    // Every file re-exports the next two and the first: a search that
    // follows each path separately takes exponential time.
    const FILES: usize = 40;
    let dir = TempDir::new().unwrap();
    for i in 0..FILES {
        let mut source = String::from("export * from './f0';\n");
        for next in [i + 1, i + 2] {
            if next < FILES {
                writeln!(source, "export * from './f{next}';").unwrap();
            }
        }
        if i == FILES - 1 {
            source.push_str(
                "import { NgModule } from '@angular/core';\n@NgModule({}) export class EndModule {}\n",
            );
        }
        write(&dir, &format!("src/f{i}.ts"), &source);
    }
    let source = r"
import { Component, NgModule } from '@angular/core';
import { EndModule, Missing } from './f0';
@Component({ selector: 'app', template: '', standalone: false }) export class App {}
@NgModule({ declarations: [App], imports: [EndModule] }) export class AModule {}
@NgModule({ declarations: [App], imports: [Missing] }) export class BModule {}
";
    let result = compile(&dir, "src/app.ts", source, &partial());
    let code = compact(&result.code);
    assert!(code.contains("ɵɵsetComponentScope(App,()=>[App],()=>[]);"), "{}", result.code);
    assert_eq!(code.matches("ɵɵsetComponentScope").count(), 1, "{}", result.code);
}

#[test]
fn a_workspace_library_class_its_entry_point_does_not_export_is_imported_by_path() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@ws/lib": ["libs/lib/index.ts"] } } }"#,
    );
    write(&dir, "libs/lib/index.ts", "export { LibModule } from './lib.module';\n");
    write(
        &dir,
        "libs/lib/lib.module.ts",
        r"
import { Directive, NgModule } from '@angular/core';
@Directive({ selector: '[lib]', standalone: false }) export class LibDir {}
@NgModule({ declarations: [LibDir], exports: [LibDir] }) export class LibModule {}
",
    );
    let options =
        TransformOptions { tsconfig_path: Some(dir.path().join("tsconfig.json")), ..partial() };
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, NgModule } from '@angular/core';
import { LibModule } from '@ws/lib';
@Component({ selector: 'app', template: '<i lib></i>', standalone: false }) export class App {}
@NgModule({ declarations: [App], imports: [LibModule] }) export class AppModule {}
",
        &options,
    );
    let code = compact(&result.code);
    assert!(code.contains("import*asi1from'../libs/lib/lib.module';"), "{}", result.code);
    assert!(
        code.contains("ɵɵsetComponentScope(App,()=>[i1.LibDir,App],()=>[]);"),
        "{}",
        result.code
    );
}

#[test]
fn a_module_with_an_unnamed_pipe_hides_none_of_its_classes() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "src/shared.ts",
        r"
import { Directive, NgModule, Pipe } from '@angular/core';
declare function pipeName(): string;
@Directive({ selector: '[s]' }) export class S {}
@Pipe({ name: pipeName() }) export class Q { transform(v: unknown) { return v; } }
@NgModule({ imports: [S, Q], exports: [S, Q] }) export class AModule {}
@NgModule({ imports: [S], exports: [S] }) export class BModule {}
",
    );
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component } from '@angular/core';
import { AModule, BModule } from './shared';
@Component({ selector: 'app', template: '<i s></i>', imports: [AModule, BModule] })
export class App {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains(
            "dependencies:[{kind:\"directive\",type:AModule,selector:\"*\"},{kind:\"ngmodule\",type:BModule},{kind:\"directive\",type:i1.S,selector:\"[s]\"}]"
        ),
        "{}",
        result.code
    );
}

/// `src/app.module.ts` declaring `C` from `src/c.ts`, whose source is `c`.
fn module_declaring_c(dir: &TempDir, c: &str) -> String {
    write(dir, "src/c.ts", c);
    let result = compile(
        dir,
        "src/app.module.ts",
        r"
import { NgModule } from '@angular/core';
import { C } from './c';
export interface Config {}
export const TOKEN = 'token';
@NgModule({ declarations: [C] })
export class AppModule {}
",
        &partial(),
    );
    compact(&result.code)
}

#[test]
fn a_type_only_import_of_the_module_file_is_no_cycle() {
    for import in [
        "import type { Config } from './app.module';",
        "import { type Config } from './app.module';",
        "export type { Config } from './app.module';",
    ] {
        let dir = TempDir::new().unwrap();
        let code = module_declaring_c(
            &dir,
            &format!(
                "import {{ Component }} from '@angular/core';\n{import}\n@Component({{ selector: 'c', template: '', standalone: false }}) export class C {{}}\n"
            ),
        );
        assert!(code.contains("ɵɵsetComponentScope(i1.C,()=>[i1.C],()=>[]);"), "{import}: {code}");
    }
}

#[test]
fn a_namespace_re_export_of_the_module_file_is_a_cycle() {
    let dir = TempDir::new().unwrap();
    write(&dir, "src/reexp.ts", "export * as mod from './app.module';\n");
    let code = module_declaring_c(
        &dir,
        r"
import { Component } from '@angular/core';
import { mod } from './reexp';
@Component({ selector: 'c', template: '', standalone: false })
export class C { token = mod.TOKEN; }
",
    );
    assert!(!code.contains("ɵɵsetComponentScope"), "{code}");
}

#[test]
fn a_cycle_through_the_barrel_the_component_is_imported_from_is_one() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "tsconfig.json",
        r#"{ "compilerOptions": { "baseUrl": ".", "paths": { "@app/shared": ["src/shared/index.ts"] } } }"#,
    );
    write(&dir, "src/shared/index.ts", "export * from './shared.module';\nexport * from './c';\n");
    write(
        &dir,
        "src/shared/c.ts",
        r"
import { Component } from '@angular/core';
@Component({ selector: 'c', template: '', standalone: false }) export class C {}
",
    );
    let options =
        TransformOptions { tsconfig_path: Some(dir.path().join("tsconfig.json")), ..partial() };
    let result = compile(
        &dir,
        "src/shared/shared.module.ts",
        r"
import { NgModule } from '@angular/core';
import { C } from '@app/shared';
@NgModule({ declarations: [C], exports: [C] })
export class SharedModule {}
",
        &options,
    );
    assert!(!result.code.contains("ɵɵsetComponentScope"), "{}", result.code);
}

#[test]
fn a_class_declared_after_the_component_is_a_forward_reference() {
    let dir = TempDir::new().unwrap();
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component, Directive, NgModule, forwardRef } from '@angular/core';
@Directive({ selector: '[e]' }) export class Early {}
@NgModule({ imports: [Early, forwardRef(() => Late)], exports: [Early, forwardRef(() => Late)] })
export class M {}
@Component({ selector: 'app', template: '<i e l></i>', imports: [M] })
export class App {}
@Directive({ selector: '[l]' }) export class Late {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains(
            "dependencies:[{kind:\"ngmodule\",type:M},{kind:\"directive\",type:Early,selector:\"[e]\"},{kind:\"directive\",type:i0.forwardRef(function(){returnLate;}),selector:\"[l]\"}]"
        ),
        "{}",
        result.code
    );
}

#[test]
fn a_class_whose_file_loads_the_component_file_is_a_forward_reference() {
    let dir = TempDir::new().unwrap();
    // Loading `d.ts` runs `app.ts` before `D` exists.
    write(
        &dir,
        "src/d.ts",
        r"
import { Directive } from '@angular/core';
import { M } from './m';
import { App } from './app';
@Directive({ selector: '[d]' }) export class D {}
export const all = [M, App];
",
    );
    write(
        &dir,
        "src/m.ts",
        r"
import { NgModule, forwardRef } from '@angular/core';
import { D } from './d';
@NgModule({ imports: [forwardRef(() => D)], exports: [forwardRef(() => D)] })
export class M {}
",
    );
    let result = compile(
        &dir,
        "src/app.ts",
        r"
import { Component } from '@angular/core';
import { M } from './m';
@Component({ selector: 'app', template: '<i d></i>', imports: [M] })
export class App {}
",
        &partial(),
    );
    let code = compact(&result.code);
    assert!(
        code.contains(
            "{kind:\"directive\",type:i0.forwardRef(function(){returni1.D;}),selector:\"[d]\"}"
        ),
        "{}",
        result.code
    );
}

#[test]
fn a_class_imported_by_path_from_an_mts_file_is_imported_as_mjs() {
    let dir = TempDir::new().unwrap();
    write(
        &dir,
        "src/d.mts",
        r"
import { Directive, NgModule } from '@angular/core';
@Directive({ selector: '[d]' }) export class D {}
",
    );
    write(
        &dir,
        "src/m.mts",
        r"
import { NgModule } from '@angular/core';
import { D } from './d.mjs';
@NgModule({ imports: [D], exports: [D] }) export class M {}
",
    );
    let result = compile(
        &dir,
        "src/app.mts",
        r"
import { Component } from '@angular/core';
import { M } from './m.mjs';
@Component({ selector: 'app', template: '<i d></i>', imports: [M] })
export class App {}
",
        &partial(),
    );
    assert!(compact(&result.code).contains("import*asi1from'./d.mjs';"), "{}", result.code);
}

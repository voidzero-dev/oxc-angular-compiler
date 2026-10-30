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
    assert!(code.contains("i0.ɵɵsetComponentScope(App,[i1.D,App],[i1.P]);"), "{}", result.code);
    // After the module's declarations: it names the component.
    let module = code.find("ɵɵngDeclareNgModule").unwrap();
    assert!(code.find("ɵɵsetComponentScope").unwrap() > module, "{}", result.code);
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
        code.contains("i0.ɵɵsetComponentScope(App,[i1.UiDir,i1.InnerDir,App],[i1.UiPipe]);"),
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
    assert!(code.contains("i0.ɵɵsetComponentScope(A,[i1.D,A,B],[i1.P]);"), "{}", result.code);
    assert!(code.contains("i0.ɵɵsetComponentScope(B,[i1.D,A,B],[i1.P]);"), "{}", result.code);
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
}

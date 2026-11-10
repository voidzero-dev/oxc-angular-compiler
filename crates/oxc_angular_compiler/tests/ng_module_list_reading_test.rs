//! NgModule lists ngtsc reads statically and Oxc used to keep as written
//! (#703): `forwardRef` targets other than a plain name, namespace
//! members, `const` lists behind a `forwardRef`, and a package's
//! `declare const` tuple. With cross-file analysis (what the Vite plugin
//! uses), the partial `ɵɵngDeclareNgModule`, the full-mode
//! `ɵɵsetNgModuleScope` and the `.d.ts` list the classes like ngtsc.
//!
//! Each expected string is what `@angular/compiler-cli` 22.2.4 writes for
//! the same files. The injector keeps its entries as written.

#![cfg(feature = "cross_file_elision")]

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};
use tempfile::TempDir;

const DEP: &str = r"
import { Directive, NgModule, ModuleWithProviders } from '@angular/core';
@Directive({ selector: '[h]', standalone: false }) export class H {}
@NgModule({}) export class DepModule { static forRoot() { return { ngModule: DepModule, providers: [] }; } }
@NgModule({}) export class TypedModule { static forRoot(x?: any): ModuleWithProviders<TypedModule> { if (x) {} return { ngModule: TypedModule, providers: [] }; } }
export class Holder { static forRoot(): ModuleWithProviders<DepModule> { const r = { ngModule: DepModule }; return r; } }
@NgModule({}) export class OtherModule {}
export const DEP_LIST = [DepModule, OtherModule];
export const MWP = DepModule.forRoot();
";

const DEP2: &str = r"
import { NgModule, ModuleWithProviders } from '@angular/core';
import { DepModule } from './dep';
@NgModule({}) export class Dep2Module { static forRoot(): ModuleWithProviders<Dep2Module> { return { ngModule: Dep2Module }; } }
export const LIST2 = [DepModule, Dep2Module];
";

const FAKELIB: &str = r"
import * as i0 from '@angular/core';
import { ModuleWithProviders } from '@angular/core';
export declare class LibModule {
  static forRoot(cfg?: any): ModuleWithProviders<LibModule>;
  static ɵfac: i0.ɵɵFactoryDeclaration<LibModule, never>;
  static ɵmod: i0.ɵɵNgModuleDeclaration<LibModule, never, never, never>;
  static ɵinj: i0.ɵɵInjectorDeclaration<LibModule>;
}
export declare class LibTwoModule {
  static ɵfac: i0.ɵɵFactoryDeclaration<LibTwoModule, never>;
  static ɵmod: i0.ɵɵNgModuleDeclaration<LibTwoModule, never, never, never>;
  static ɵinj: i0.ɵɵInjectorDeclaration<LibTwoModule>;
}
export declare const LIB_ALL: [typeof LibModule, typeof LibTwoModule];
";

fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
    let path = dir.path().join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, content).unwrap();
    path
}

fn project() -> TempDir {
    let dir = TempDir::new().unwrap();
    write(&dir, "src/dep.ts", DEP);
    write(&dir, "src/dep2.ts", DEP2);
    write(
        &dir,
        "node_modules/fakelib/package.json",
        r#"{ "name": "fakelib", "types": "index.d.ts" }"#,
    );
    write(&dir, "node_modules/fakelib/index.d.ts", FAKELIB);
    dir
}

fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The full and partial output of `src/m.ts`, and the `.d.ts` members of
/// its class `M`, all without whitespace.
struct Out {
    full: String,
    partial: String,
    dts: String,
}

fn compile(source: &str) -> Out {
    let dir = project();
    let path = write(&dir, "src/m.ts", source);
    let run = |mode| {
        let allocator = Allocator::default();
        let options = TransformOptions {
            compilation_mode: mode,
            resolve_imported_values: true,
            ..Default::default()
        };
        let result = transform_angular_file(
            &allocator,
            path.to_str().unwrap(),
            source,
            Some(&options),
            None,
        );
        assert!(!result.has_errors(), "{:?}", result.diagnostics);
        let dts = result
            .dts_declarations
            .iter()
            .find(|decl| decl.class_name == "M")
            .map(|decl| compact(&decl.members))
            .unwrap_or_default();
        (compact(&result.code), dts)
    };
    let (full, dts) = run(CompilationMode::Full);
    let (partial, _) = run(CompilationMode::Partial);
    Out { full, partial, dts }
}

/// Checks one module: the namespace imports, the full scope, the partial
/// lists and the `.d.ts` lists, as ngtsc writes them (whitespace removed).
fn check(source: &str, imports: &str, scope: &str, partial: &str, dts: &str) {
    let out = compile(source);
    for code in [&out.full, &out.partial] {
        assert!(code.contains(imports), "imports {imports}:\n{code}");
    }
    assert!(out.full.contains(&format!("ɵɵsetNgModuleScope(M,{scope})")), "scope:\n{}", out.full);
    assert!(out.partial.contains(&format!("type:M,{partial}}}")), "partial:\n{}", out.partial);
    assert!(out.dts.contains(&format!("ɵɵNgModuleDeclaration<M,{dts}>")), "dts:\n{}", out.dts);
}

const H: &str = "import { NgModule, forwardRef } from '@angular/core';\n";

#[test]
fn issue_example_namespace_forward_ref_and_method() {
    let source = format!(
        "{H}import * as dep from './dep';
@NgModule({{ imports: [forwardRef(() => dep.DepModule), dep.DepModule.forRoot()] }}) export class M {{}}"
    );
    check(
        &source,
        "import*asi1from'./dep';",
        "{imports:[i1.DepModule,i1.DepModule]}",
        "imports:[i1.DepModule,i1.DepModule]",
        "never,[typeofi1.DepModule,typeofi1.DepModule],never",
    );
    let full = compile(&source).full;
    assert!(
        full.contains(
            "ɵɵdefineInjector({imports:[forwardRef(()=>dep.DepModule),dep.DepModule.forRoot()]})"
        ),
        "{full}"
    );
}

#[test]
fn issue_example_spread_and_const_list() {
    let source = format!(
        "{H}import {{ DepModule }} from './dep';
const LIST = [DepModule];
@NgModule({{ imports: [...LIST], exports: LIST }}) export class M {{}}"
    );
    check(
        &source,
        "import*asi0from'@angular/core';",
        "{imports:[DepModule],exports:[DepModule]}",
        "imports:[DepModule],exports:[DepModule]",
        "never,[typeofi1.DepModule],[typeofi1.DepModule]",
    );
    let full = compile(&source).full;
    assert!(full.contains("ɵɵdefineInjector({imports:[LIST,DepModule]})"), "{full}");
}

#[test]
fn forward_ref_function_shapes() {
    // ngtsc's `expandForwardRef`: an arrow or function expression, with an
    // expression body or one `return`; parentheses and `as` around it are
    // looked through, and so is an aliased import of `forwardRef`.
    for imports in [
        "forwardRef(function () { return dep.DepModule; })",
        "forwardRef(() => { return dep.DepModule; })",
        "forwardRef((() => dep.DepModule))",
        "forwardRef((() => dep.DepModule) as any)",
        "forwardRef(() => (dep.DepModule as any))",
        "forwardRef(() => dep['DepModule'])",
        "fr(() => dep.DepModule)",
    ] {
        let source = format!(
            "import {{ NgModule, forwardRef, forwardRef as fr }} from '@angular/core';
import * as dep from './dep';
@NgModule({{ imports: [{imports}] }}) export class M {{}}"
        );
        let out = compile(&source);
        assert!(
            out.full.contains("ɵɵsetNgModuleScope(M,{imports:[i1.DepModule]})"),
            "{imports}:\n{}",
            out.full
        );
        assert!(
            out.partial.contains("type:M,imports:[i1.DepModule]}"),
            "{imports}:\n{}",
            out.partial
        );
        assert!(
            out.dts.contains("<M,never,[typeofi1.DepModule],never>"),
            "{imports}:\n{}",
            out.dts
        );
    }
}

#[test]
fn forward_ref_to_const_lists() {
    // A local list, an imported one, and a package's tuple type.
    check(
        &format!(
            "{H}import {{ DepModule }} from './dep';
const LIST = [DepModule];
@NgModule({{ imports: [forwardRef(() => LIST)] }}) export class M {{}}"
        ),
        "import*asi0from'@angular/core';",
        "{imports:[DepModule]}",
        "imports:[DepModule]",
        "never,[typeofi1.DepModule],never",
    );
    check(
        &format!(
            "{H}import {{ DEP_LIST }} from './dep';
@NgModule({{ imports: [forwardRef(() => DEP_LIST)] }}) export class M {{}}"
        ),
        "import*asi1from'./dep';",
        "{imports:[i1.DepModule,i1.OtherModule]}",
        "imports:[i1.DepModule,i1.OtherModule]",
        "never,[typeofi1.DepModule,typeofi1.OtherModule],never",
    );
    check(
        &format!(
            "{H}import {{ LIB_ALL }} from 'fakelib';
@NgModule({{ imports: [forwardRef(() => LIB_ALL)] }}) export class M {{}}"
        ),
        "import*asi1from'fakelib';",
        "{imports:[i1.LibModule,i1.LibTwoModule]}",
        "imports:[i1.LibModule,i1.LibTwoModule]",
        "never,[typeofi1.LibModule,typeofi1.LibTwoModule],never",
    );
}

#[test]
fn namespace_members_in_every_list() {
    check(
        &format!(
            "{H}import * as dep from './dep';
@NgModule({{ imports: [dep.DepModule], declarations: [forwardRef(() => dep.H)], exports: [forwardRef(() => dep.H), dep.DepModule] }}) export class M {{}}"
        ),
        "import*asi1from'./dep';",
        "{declarations:[i1.H],imports:[i1.DepModule],exports:[i1.H,i1.DepModule]}",
        "declarations:[i1.H],imports:[i1.DepModule],exports:[i1.H,i1.DepModule]",
        "[typeofi1.H],[typeofi1.DepModule],[typeofi1.H,typeofi1.DepModule]",
    );
}

#[test]
fn full_mode_scope_numbers_namespaces_before_the_injector() {
    // ngtsc numbers `./dep` and `./dep2` in the scope, before ɵinj.
    let source = format!(
        "{H}import * as d2 from './dep2';
import {{ LIST2 }} from './dep2';
@NgModule({{ imports: [...LIST2, d2.Dep2Module.forRoot()], exports: [d2.Dep2Module] }}) export class M {{}}"
    );
    check(
        &source,
        "import*asi1from'./dep';import*asi2from'./dep2';",
        "{imports:[i1.DepModule,i2.Dep2Module,i2.Dep2Module],exports:[i2.Dep2Module]}",
        "imports:[i1.DepModule,i2.Dep2Module,i2.Dep2Module],exports:[i2.Dep2Module]",
        "never,[typeofi1.DepModule,typeofi2.Dep2Module,typeofi2.Dep2Module],[typeofi2.Dep2Module]",
    );
    let full = compile(&source).full;
    assert!(
        full.contains("ɵɵdefineInjector({imports:[LIST2,d2.Dep2Module.forRoot(),i2.Dep2Module]})"),
        "{full}"
    );
}

#[test]
fn a_package_tuple_takes_its_namespace_first() {
    let source = format!(
        "{H}import {{ LIB_ALL }} from 'fakelib';
import * as d2 from './dep2';
@NgModule({{ imports: [...LIB_ALL, d2.Dep2Module], exports: [d2.Dep2Module] }}) export class M {{}}"
    );
    check(
        &source,
        "import*asi1from'fakelib';import*asi2from'./dep2';",
        "{imports:[i1.LibModule,i1.LibTwoModule,i2.Dep2Module],exports:[i2.Dep2Module]}",
        "imports:[i1.LibModule,i1.LibTwoModule,i2.Dep2Module],exports:[i2.Dep2Module]",
        "never,[typeofi1.LibModule,typeofi1.LibTwoModule,typeofi2.Dep2Module],[typeofi2.Dep2Module]",
    );
}

#[test]
fn module_with_providers_calls() {
    // The NgModule each call gives, named through the file that declares
    // it, by name, through a namespace, and from a package.
    check(
        &format!(
            "{H}import {{ DepModule, TypedModule, Holder }} from './dep';
@NgModule({{ imports: [DepModule.forRoot(), TypedModule.forRoot(), Holder.forRoot()] }}) export class M {{}}"
        ),
        "import*asi1from'./dep';",
        "{imports:[i1.DepModule,i1.TypedModule,i1.DepModule]}",
        "imports:[i1.DepModule,i1.TypedModule,i1.DepModule]",
        "never,[typeofi1.DepModule,typeofi1.TypedModule,typeofi1.DepModule],never",
    );
    check(
        &format!(
            "{H}import * as dep from './dep';
@NgModule({{ imports: [dep.TypedModule.forRoot(), dep.Holder.forRoot()] }}) export class M {{}}"
        ),
        "import*asi1from'./dep';",
        "{imports:[i1.TypedModule,i1.DepModule]}",
        "imports:[i1.TypedModule,i1.DepModule]",
        "never,[typeofi1.TypedModule,typeofi1.DepModule],never",
    );
    check(
        &format!(
            "{H}import {{ LibModule }} from 'fakelib';
import * as lib from 'fakelib';
@NgModule({{ imports: [LibModule.forRoot({{}}), lib.LibModule.forRoot(), forwardRef(() => lib.LibModule)], exports: [lib.LibModule] }}) export class M {{}}"
        ),
        "import*asi1from'fakelib';",
        "{imports:[i1.LibModule,i1.LibModule,i1.LibModule],exports:[i1.LibModule]}",
        "imports:[i1.LibModule,i1.LibModule,i1.LibModule],exports:[i1.LibModule]",
        "never,[typeofi1.LibModule,typeofi1.LibModule,typeofi1.LibModule],[typeofi1.LibModule]",
    );
}

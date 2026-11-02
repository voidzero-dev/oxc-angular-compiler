//! `ModuleWithProviders` entries in a partial `ɵɵngDeclareNgModule`'s
//! `imports` (#623). ngtsc lists the NgModule each one names
//! (`resolveTypeList`): `imports: [mwp, Holder.forRoot()]` becomes
//! `imports: [DepModule, DepModule]`. The injector and the full-mode
//! `ɵɵsetNgModuleScope` keep the entries as written.
//!
//! Each expected list below was checked against ngtsc 22.2.3 on the same
//! input. An entry oxc can't settle keeps its old output.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

fn compact(code: &str) -> String {
    code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile_in(path: &str, source: &str, options: &TransformOptions) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, path, source, Some(options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    compact(&result.code)
}

fn partial(source: &str) -> String {
    let options =
        TransformOptions { compilation_mode: CompilationMode::Partial, ..Default::default() };
    compile_in("app.ts", source, &options)
}

/// ngtsc partial: `imports: [DepModule, DepModule]`; the injector keeps
/// `[mwp, Holder.forRoot()]`.
const SAME_FILE: &str = r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class DepModule {}
export class Holder {
  static forRoot(): ModuleWithProviders<DepModule> { return { ngModule: DepModule, providers: [] }; }
}
const mwp = Holder.forRoot();
@NgModule({ imports: [mwp, Holder.forRoot()] })
export class AppModule {}
";

#[test]
fn same_file_method_and_constant_list_the_module() {
    let code = partial(SAME_FILE);
    assert!(
        code.contains("type:AppModule,imports:[DepModule,DepModule]}"),
        "ɵmod should list DepModule twice:\n{code}"
    );
    assert!(
        code.contains("type:AppModule,imports:[mwp,Holder.forRoot()]}"),
        "ɵinj should keep the entries as written:\n{code}"
    );
}

#[test]
fn full_mode_scope_keeps_the_entries_as_written() {
    // ngtsc's full output reads `[DepModule, DepModule]` here, but the
    // runtime unwraps the raw list (`convertToTypeArray`), like ngtsc's
    // local compilation. Full output doesn't change.
    let options = TransformOptions::default();
    let code = compile_in("app.ts", SAME_FILE, &options);
    assert!(
        code.contains("ɵɵsetNgModuleScope(AppModule,{imports:[mwp,Holder.forRoot()]})"),
        "full-mode scope should be raw:\n{code}"
    );
}

#[test]
fn reads_the_method_in_ngtsc_order() {
    // ngtsc evaluates a body that is a single `return` first, so `both`
    // gives BModule; any other body falls back to the return type
    // (`multi` gives CModule); with no return type the body still counts
    // (`plain` gives AModule). Constants follow the same rules.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders, InjectionToken } from '@angular/core';
export const TOK = new InjectionToken<string>('tok');
@NgModule({})
export class AModule {}
@NgModule({})
export class BModule {}
@NgModule({})
export class CModule {}
export class Holder {
  static both(): ModuleWithProviders<AModule> { return { ngModule: BModule, providers: [] }; }
  static multi(v: string): ModuleWithProviders<CModule> {
    const providers = [{ provide: TOK, useValue: v }];
    return { ngModule: CModule, providers };
  }
  static plain() { return { ngModule: AModule, providers: [] }; }
}
const lit: ModuleWithProviders<BModule> = { ngModule: BModule, providers: [] };
const viaCall = Holder.multi('x');
@NgModule({ imports: [Holder.both(), Holder.multi('y'), Holder.plain(), lit, viaCall, AModule] })
export class AppModule {}
",
    );
    assert!(
        code.contains("type:AppModule,imports:[BModule,CModule,AModule,BModule,CModule,AModule]}"),
        "{code}"
    );
}

#[test]
fn local_method_names_an_imported_module_as_written() {
    // The module comes from this file's own text, so ngtsc names it as
    // written, through the existing import.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
import { DepModule } from './dep';
export class LocalHolder {
  static typed(): ModuleWithProviders<DepModule> { const p: any[] = []; return { ngModule: DepModule, providers: p }; }
  static body() { return { ngModule: DepModule, providers: [] }; }
}
@NgModule({ imports: [LocalHolder.typed(), LocalHolder.body()] })
export class AppModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:[DepModule,DepModule]}"), "{code}");
    assert!(code.contains("import{DepModule}from'./dep';"), "{code}");
}

#[test]
fn unsure_entries_keep_their_old_output() {
    // - `later`: the module is declared after AppModule, so naming it would
    //   read it before it exists (ngtsc wraps the list in `() => [...]`).
    // - `typeOnly`: `DepModule` is only a type here, so its import is
    //   removed; ngtsc names it anyway and drops the import.
    // - `chained`: the body returns another call, which ngtsc evaluates
    //   further.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
import { DepModule } from './dep';
export class Holder {
  static later(): ModuleWithProviders<LaterModule> { const p: any[] = []; return { ngModule: LaterModule, providers: p }; }
  static typeOnly(): ModuleWithProviders<DepModule> { const p: any[] = []; return { ngModule: null as any, providers: p }; }
  static chained() { return Other.forRoot(); }
}
export class Other { static forRoot(): ModuleWithProviders<AModule> { return { ngModule: AModule, providers: [] }; } }
@NgModule({})
export class AModule {}
@NgModule({ imports: [Holder.later(), Holder.typeOnly(), Holder.chained()] })
export class AppModule {}
@NgModule({})
export class LaterModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:[Holder,Holder,Holder]}"), "{code}");
    assert!(!code.contains("'./dep'"), "the type-only import stays removed:\n{code}");
}

#[test]
fn a_parameter_or_type_parameter_keeps_its_old_output() {
    // The module comes from the call's argument or type argument, which
    // oxc doesn't follow.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders, Type } from '@angular/core';
@NgModule({})
export class AModule {}
export class Holder {
  static of(ngModule: Type<any>) { return { ngModule, providers: [] }; }
  static generic<T>(): ModuleWithProviders<T> { const m: any = null; return m; }
}
@NgModule({ imports: [Holder.of(AModule), Holder.generic<AModule>()] })
export class AppModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:[Holder,Holder]}"), "{code}");
}

#[test]
fn a_class_in_a_function_keeps_its_old_output() {
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class DepModule {}
export class Holder {
  static forRoot(): ModuleWithProviders<DepModule> { return { ngModule: DepModule, providers: [] }; }
}
export function make() {
  @NgModule({ imports: [Holder.forRoot()] })
  class AppModule {}
  return AppModule;
}
",
    );
    assert!(code.contains("type:AppModule,imports:[Holder]}"), "{code}");
}

#[test]
fn an_imported_holder_needs_cross_file_analysis() {
    // Without `resolve_imported_values`, oxc can't read another file.
    let code = partial(
        r"
import { NgModule } from '@angular/core';
import { OtherModuleHolder, mwp } from './dep';
@NgModule({ imports: [mwp, OtherModuleHolder.forRoot()] })
export class AppModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:[mwp,OtherModuleHolder]}"), "{code}");
}

#[cfg(feature = "cross_file_elision")]
mod cross_file {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn compile(dir: &TempDir, name: &str, source: &str, mode: CompilationMode) -> String {
        let path = write(dir, name, source);
        let options = TransformOptions {
            compilation_mode: mode,
            resolve_imported_values: true,
            ..Default::default()
        };
        compile_in(path.to_str().unwrap(), source, &options)
    }

    const DEP: &str = r"
import { NgModule, ModuleWithProviders, Component } from '@angular/core';
@Component({ selector: 'dep-c', template: 'd', standalone: false })
export class DepCmp {}
@NgModule({ declarations: [DepCmp], exports: [DepCmp] })
export class DepModule {
  static forRoot(): ModuleWithProviders<DepModule> {
    return { ngModule: DepModule, providers: [] };
  }
}
export class OtherModuleHolder {
  static forRoot(): ModuleWithProviders<DepModule> { return { ngModule: DepModule, providers: [] }; }
}
export const mwp = DepModule.forRoot();
";

    const APP: &str = r"
import { NgModule, Component } from '@angular/core';
import { OtherModuleHolder, mwp } from './dep';
@Component({ selector: 'app-c', template: '<dep-c/>', standalone: false })
export class AppCmp {}
@NgModule({ declarations: [AppCmp], imports: [mwp, OtherModuleHolder.forRoot()] })
export class AppModule {}
";

    #[test]
    fn imported_method_and_constant_name_the_module_through_an_import() {
        // ngtsc partial: `import * as i1 from "./dep"` and
        // `imports: [i1.DepModule, i1.DepModule]`.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP);
        let code = compile(&dir, "app.ts", APP, CompilationMode::Partial);
        assert!(code.contains("import*asi1from'./dep';"), "{code}");
        assert!(
            code.contains("declarations:[AppCmp],imports:[i1.DepModule,i1.DepModule]}"),
            "{code}"
        );
        assert!(
            code.contains("type:AppModule,imports:[mwp,OtherModuleHolder.forRoot()]}"),
            "ɵinj should keep the entries as written:\n{code}"
        );
    }

    #[test]
    fn full_mode_scope_stays_raw_with_cross_file_analysis() {
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP);
        let code = compile(&dir, "app.ts", APP, CompilationMode::Full);
        assert!(
            code.contains(
                "ɵɵsetNgModuleScope(AppModule,{declarations:[AppCmp],imports:[mwp,OtherModuleHolder.forRoot()]})"
            ),
            "{code}"
        );
    }

    #[test]
    fn a_class_imported_by_name_is_kept_as_written() {
        // ngtsc partial: `[DepModule, i1.DepModule, i1.DepModule, i1.DepModule]`:
        // a plain entry stays as written, and an NgModule found in another
        // file is named through `i1` even when the file imports it by name.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP);
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule, Component } from '@angular/core';
import { DepModule, OtherModuleHolder, mwp } from './dep';
@Component({ selector: 'app-c', template: '<dep-c/>', standalone: false })
export class AppCmp {}
@NgModule({ declarations: [AppCmp], imports: [DepModule, mwp, OtherModuleHolder.forRoot(), DepModule.forRoot()] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(
            code.contains("imports:[DepModule,i1.DepModule,i1.DepModule,i1.DepModule]}"),
            "{code}"
        );
    }

    #[test]
    fn a_declaration_file_method_reads_its_return_type() {
        // ngtsc partial: `import * as i1 from "./lib"`, `imports: [i1.LibModule]`.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "lib.d.ts",
            r"
import * as i0 from '@angular/core';
import { ModuleWithProviders } from '@angular/core';
export declare class LibModule {
  static forRoot(): ModuleWithProviders<LibModule>;
  static ɵfac: i0.ɵɵFactoryDeclaration<LibModule, never>;
  static ɵmod: i0.ɵɵNgModuleDeclaration<LibModule, never, never, never>;
  static ɵinj: i0.ɵɵInjectorDeclaration<LibModule>;
}
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { LibModule } from './lib';
@NgModule({ imports: [LibModule.forRoot()] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("import*asi1from'./lib';"), "{code}");
        assert!(code.contains("type:AppModule,imports:[i1.LibModule]}"), "{code}");
    }

    #[test]
    fn a_local_constant_of_an_imported_method() {
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP);
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { OtherModuleHolder } from './dep';
const local = OtherModuleHolder.forRoot();
@NgModule({ imports: [local] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("type:AppModule,imports:[i1.DepModule]}"), "{code}");
    }

    #[test]
    fn an_unreadable_entry_keeps_its_old_output() {
        // No return type and a body ngtsc can't evaluate, and a method that
        // names a class which is no NgModule.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dep.ts",
            r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class DepModule {}
export class NotAModule {}
export class Holder {
  static loose() { const p: any[] = []; return { ngModule: DepModule, providers: p }; }
  static wrong(): ModuleWithProviders<NotAModule> { return { ngModule: NotAModule, providers: [] }; }
}
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { Holder, DepModule } from './dep';
@NgModule({ imports: [Holder.loose(), Holder.wrong(), DepModule] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("type:AppModule,imports:[Holder,Holder,DepModule]}"), "{code}");
        assert!(!code.contains("import*asi1"), "{code}");
    }
}

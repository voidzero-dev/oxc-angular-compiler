//! `ɵinj` imports of an NgModule (#623). In full and partial compilation,
//! ngtsc's `NgModuleDecoratorHandler.resolve` leaves the directives and pipes
//! of `imports` out of the injector, and of `exports` also the components:
//! the runtime's `walkProviderTree` skips them. Oxc does the same when it
//! knows what an entry is: from this file alone, or, with cross-file
//! analysis (`TransformOptions::resolve_imported_values`), from the files and
//! packages it imports. Anything else stays as written.
//!
//! Each expected output was checked against ngtsc 22.2.3. Where Oxc still
//! differs, the test says so: ngtsc also drops a standalone component from
//! `imports` when it brings in no providers (`mayExportProviders`), and
//! rewrites a list that is only partly dropped into the references it keeps.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

fn options(mode: CompilationMode) -> TransformOptions {
    TransformOptions { compilation_mode: mode, ..Default::default() }
}

/// Each `ɵinj` initializer of `code`, whitespace removed, in order, with
/// the partial declaration's `version` left out.
fn injectors(code: &str) -> Vec<String> {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    compact
        .split("ɵinj=")
        .skip(1)
        .map(|rest| {
            let init = &rest[..rest.find(';').unwrap()];
            match init.find("version:\"") {
                Some(start) => {
                    let end = start + 9 + init[start + 9..].find('"').unwrap() + 2;
                    format!("{}{}", &init[..start], &init[end..])
                }
                None => init.to_string(),
            }
        })
        .collect()
}

fn compile_injectors(path: &str, source: &str, options: &TransformOptions) -> Vec<String> {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, path, source, Some(options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    injectors(&result.code)
}

/// The full and the partial `ɵinj` of a module, from its `imports` list
/// (`None` for no `imports` key).
fn both(name: &str, imports: Option<&str>) -> [String; 2] {
    let full = match imports {
        Some(list) => format!("/*@__PURE__*/i0.ɵɵdefineInjector({{imports:[{list}]}})"),
        None => "/*@__PURE__*/i0.ɵɵdefineInjector({})".to_string(),
    };
    let head = format!("i0.ɵɵngDeclareInjector({{minVersion:\"12.0.0\",ngImport:i0,type:{name}");
    let partial = match imports {
        Some(list) => format!("{head},imports:[{list}]}})"),
        None => format!("{head}}})"),
    };
    [full, partial]
}

/// Check the modules of `source`, compiled as `path`, in full and partial
/// compilation; `expected` is `(module, imports)` per module, in order.
fn assert_injectors_at(
    path: &str,
    source: &str,
    expected: &[(&str, Option<&str>)],
    base: &TransformOptions,
) {
    for (index, mode) in [CompilationMode::Full, CompilationMode::Partial].into_iter().enumerate() {
        let options = TransformOptions { compilation_mode: mode, ..base.clone() };
        let got = compile_injectors(path, source, &options);
        let want: Vec<String> =
            expected.iter().map(|(name, imports)| both(name, *imports)[index].clone()).collect();
        assert_eq!(got, want, "{mode:?}");
    }
}

fn assert_injectors(source: &str, expected: &[(&str, Option<&str>)], base: &TransformOptions) {
    assert_injectors_at("app.ts", source, expected, base);
}

#[test]
fn exports_that_are_also_declared_are_left_out() {
    // ngtsc: `ɵɵdefineInjector({})`, and no `imports` in partial.
    assert_injectors(
        r"
import { Directive, NgModule } from '@angular/core';
import { Other } from './other';
@Directive({ selector: '[ndir]', standalone: false }) export class NonStdDir {}
@NgModule({ declarations: [NonStdDir, Other], exports: [NonStdDir, Other] }) export class DirModule {}
",
        &[("DirModule", None)],
        &TransformOptions::default(),
    );
}

#[test]
fn directives_and_pipes_of_this_file_are_left_out() {
    // ngtsc gives `{}` for both: it also drops `LocalCmp`, a standalone
    // component that brings in no providers. Oxc keeps components in
    // `imports`.
    assert_injectors(
        r"
import { Directive, Pipe, NgModule, Component } from '@angular/core';
@Directive({ selector: '[sdir]' }) export class StandaloneDir {}
@Pipe({ name: 'spipe' }) export class StandalonePipe { transform(v: any) { return v; } }
@Component({ selector: 'lc', template: '' }) export class LocalCmp {}
@NgModule({ imports: [StandaloneDir, StandalonePipe], exports: [StandaloneDir, StandalonePipe] }) export class ReexportModule {}
@NgModule({ imports: [StandaloneDir, StandalonePipe, LocalCmp], exports: [StandaloneDir] }) export class LocalModule {}
",
        &[("ReexportModule", None), ("LocalModule", Some("LocalCmp"))],
        &TransformOptions::default(),
    );
}

#[test]
fn forward_refs_are_read_through() {
    // ngtsc: `{ imports: [forwardRef(() => LateModule)] }` and `{}`.
    assert_injectors(
        r"
import { Directive, NgModule, forwardRef } from '@angular/core';
@Directive({ selector: '[decl]', standalone: false }) export class DeclDir {}
@NgModule({
  imports: [forwardRef(() => LateDir), forwardRef(() => LateModule)],
  declarations: [DeclDir],
  exports: [forwardRef(() => DeclDir)],
})
export class FwdModule {}
@Directive({ selector: '[late]' }) export class LateDir {}
@NgModule({}) export class LateModule {}
",
        &[("FwdModule", Some("forwardRef(()=>LateModule)")), ("LateModule", None)],
        &TransformOptions::default(),
    );
}

#[test]
fn aliased_and_namespaced_decorators_are_recognized() {
    // ngtsc: `{}` for LateModule, `{ imports: [LateModule, LateModule] }`.
    assert_injectors(
        r"
import { Directive as D, NgModule } from '@angular/core';
import * as ng from '@angular/core';
@D({ selector: '[alias]' }) export class AliasDir {}
@ng.Pipe({ name: 'nsp' }) export class NsPipe { transform(v: any) { return v; } }
@NgModule({}) export class LateModule {}
@NgModule({ imports: [AliasDir, NsPipe, LateModule], exports: [AliasDir, NsPipe, LateModule] })
export class AliasModule {}
",
        &[("LateModule", None), ("AliasModule", Some("LateModule,LateModule"))],
        &TransformOptions::default(),
    );
}

#[test]
fn an_exported_component_stays_unless_imported_or_declared() {
    // ngtsc drops `Cmp` from `exports` and, as it has no providers, from
    // `imports` too. A component exported without being imported or declared
    // is an error in ngtsc; Oxc keeps it, as its providers may be needed.
    assert_injectors(
        r"
import { Component, NgModule } from '@angular/core';
@Component({ selector: 'c', template: '' }) export class Cmp {}
@Component({ selector: 'o', template: '' }) export class Orphan {}
@NgModule({ imports: [Cmp], exports: [Cmp, Orphan] }) export class M {}
",
        &[("M", Some("Cmp,Orphan"))],
        &TransformOptions::default(),
    );
}

#[test]
fn what_is_not_known_to_be_a_directive_or_pipe_stays() {
    // A decorator not imported from `@angular/core`, an imported class (no
    // cross-file analysis), a `forRoot()` call, a const list, a spread and
    // a `forwardRef` that is not Angular's all stay as written.
    assert_injectors(
        r"
import { NgModule } from '@angular/core';
import { Directive } from './my-decorators';
import { forwardRef } from './my-forward-ref';
import { ImportedDir, RouterModule, LIST } from './lib';
@Directive({ selector: '[x]' }) export class NotAngular {}
@NgModule({
  imports: [NotAngular, ImportedDir, RouterModule.forRoot([]), LIST, ...LIST, forwardRef(() => NotAngular)],
  exports: [ImportedDir, Unknown],
})
export class M {}
",
        &[(
            "M",
            Some(
                "NotAngular,ImportedDir,RouterModule.forRoot([]),LIST,...LIST,forwardRef(()=>NotAngular),ImportedDir,Unknown",
            ),
        )],
        &TransformOptions::default(),
    );
}

#[test]
fn a_module_in_a_function_keeps_its_lists() {
    // A nested class could name a binding of its function, which the file's
    // top-level classes don't show, so its lists stay as written.
    let got = compile_injectors(
        "app.ts",
        r"
import { Directive, NgModule } from '@angular/core';
@Directive({ selector: '[d]' }) export class D {}
export function make() {
  @NgModule({ imports: [D] }) class Inner {}
  return Inner;
}
",
        &options(CompilationMode::Full),
    );
    assert_eq!(got, vec!["/*@__PURE__*/i0.ɵɵdefineInjector({imports:[D]})".to_string()]);
}

#[cfg(feature = "cross_file_elision")]
mod cross_file {
    //! With cross-file analysis, imported classes, `const` lists and package
    //! declaration files are read too.

    use oxc_allocator::Allocator;
    use oxc_angular_compiler::{TransformOptions, transform_angular_file};
    use tempfile::TempDir;

    use super::{assert_injectors_at, injectors};

    fn write(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn cross_file() -> TransformOptions {
        TransformOptions { resolve_imported_values: true, ..Default::default() }
    }

    /// Compile `src/app.ts` in `dir` with `options` and check its modules.
    fn check(
        dir: &TempDir,
        source: &str,
        expected: &[(&str, Option<&str>)],
        options: &TransformOptions,
    ) {
        let path = write(dir, "src/app.ts", source);
        assert_injectors_at(path.to_str().unwrap(), source, expected, options);
    }

    /// `src/dep.ts` of the ngtsc case: standalone classes and two modules.
    fn dep(dir: &TempDir) {
        write(
            dir,
            "src/dep.ts",
            r"
import { Directive, Pipe, NgModule, Component } from '@angular/core';
@Directive({ selector: '[sdir]' }) export class StandaloneDir {}
@Pipe({ name: 'spipe' }) export class StandalonePipe { transform(v: any) { return v; } }
@Component({ selector: 'scmp', template: '' }) export class StandaloneCmp {}
@Component({ selector: 'ncmp', template: '', standalone: false }) export class NonStdCmp {}
@Directive({ selector: '[ndir]', standalone: false }) export class NonStdDir {}
@NgModule({ declarations: [NonStdDir], exports: [NonStdDir] }) export class DirModule {}
@NgModule({ imports: [StandaloneDir, StandalonePipe], exports: [StandaloneDir, StandalonePipe] }) export class ReexportModule {}
@NgModule({ declarations: [NonStdCmp], exports: [NonStdCmp] }) export class CmpModule {}
@NgModule({}) export class PlainModule {}
export const DIRS = [StandaloneDir, StandalonePipe];
export const MIXED = [StandaloneDir, PlainModule];
",
        );
    }

    /// `node_modules/@lib/common`: declaration files shaped like
    /// `@angular/common`'s.
    fn common_package(dir: &TempDir) {
        write(
            dir,
            "node_modules/@lib/common/package.json",
            r#"{ "name": "@lib/common", "exports": { ".": { "types": "./types/common.d.ts", "default": "./fesm/common.mjs" } } }"#,
        );
        write(dir, "node_modules/@lib/common/fesm/common.mjs", "export {};\n");
        write(
            dir,
            "node_modules/@lib/common/types/common.d.ts",
            "export { NgIf, AsyncPipe, NgClass, CommonModule } from './_chunk.js';\n",
        );
        write(
            dir,
            "node_modules/@lib/common/types/_chunk.d.ts",
            r#"
import * as i0 from '@angular/core';
declare class NgIf {
    static ɵdir: i0.ɵɵDirectiveDeclaration<NgIf, "[ngIf]", never, {}, {}, never, never, true, never>;
}
declare class NgClass {
    static ɵdir: i0.ɵɵDirectiveDeclaration<NgClass, "[ngClass]", never, {}, {}, never, never, true, never>;
}
declare class AsyncPipe {
    static ɵpipe: i0.ɵɵPipeDeclaration<AsyncPipe, "async", true>;
}
declare class CommonModule {
    static ɵmod: i0.ɵɵNgModuleDeclaration<CommonModule, never, [typeof NgIf, typeof NgClass, typeof AsyncPipe], [typeof NgIf, typeof NgClass, typeof AsyncPipe]>;
}
export { NgIf, NgClass, AsyncPipe, CommonModule };
"#,
        );
    }

    #[test]
    fn imported_directives_and_pipes_are_left_out() {
        // ngtsc: `[DirModule, ReexportModule, LocalModule, DirModule,
        // ReexportModule, LocalModule]`: it also drops `StandaloneCmp`,
        // which brings in no providers.
        let dir = TempDir::new().unwrap();
        dep(&dir);
        check(
            &dir,
            r"
import { Directive, Pipe, NgModule } from '@angular/core';
import { StandaloneDir, StandalonePipe, StandaloneCmp, DirModule, ReexportModule } from './dep';
@Directive({ selector: '[localdir]' }) export class LocalDir {}
@Pipe({ name: 'lp' }) export class LocalPipe { transform(v: any) { return v; } }
@NgModule({ imports: [LocalDir] }) export class LocalModule {}
@NgModule({
  imports: [StandaloneDir, StandalonePipe, StandaloneCmp, DirModule, ReexportModule, LocalDir, LocalModule, LocalPipe],
  exports: [StandaloneDir, DirModule, ReexportModule, LocalModule, LocalPipe],
})
export class AppModule {}
",
            &[
                ("LocalModule", None),
                (
                    "AppModule",
                    Some(
                        "StandaloneCmp,DirModule,ReexportModule,LocalModule,DirModule,ReexportModule,LocalModule",
                    ),
                ),
            ],
            &cross_file(),
        );
    }

    #[test]
    fn namespaces_lists_and_reexported_components() {
        // ngtsc, module by module: `[dep.PlainModule]`, `{}`,
        // `[i1.PlainModule, i1.PlainModule]` (Oxc keeps a list that is only
        // partly dropped whole), and `[CmpModule, CmpModule]` (it also drops
        // `StandaloneCmp`, which brings in no providers).
        let dir = TempDir::new().unwrap();
        dep(&dir);
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import * as dep from './dep';
import { DIRS, MIXED, StandaloneCmp, NonStdCmp, CmpModule } from './dep';
@NgModule({ imports: [dep.StandaloneDir, dep.PlainModule, ...DIRS] })
export class NsModule {}
@NgModule({ imports: DIRS, exports: DIRS })
export class WholeListModule {}
@NgModule({ imports: [...MIXED], exports: MIXED })
export class MixedModule {}
@NgModule({ imports: [StandaloneCmp, CmpModule], exports: [StandaloneCmp, NonStdCmp, CmpModule] })
export class CmpReexportModule {}
",
            &[
                ("NsModule", Some("dep.PlainModule")),
                ("WholeListModule", None),
                ("MixedModule", Some("...MIXED,MIXED")),
                ("CmpReexportModule", Some("StandaloneCmp,CmpModule,CmpModule")),
            ],
            &cross_file(),
        );
    }

    #[test]
    fn aliased_imports_and_forward_refs() {
        // ngtsc: `[ReexportModule, DirModule]`.
        let dir = TempDir::new().unwrap();
        dep(&dir);
        check(
            &dir,
            r"
import { NgModule, forwardRef } from '@angular/core';
import { StandaloneDir as SD, StandalonePipe, DirModule, ReexportModule } from './dep';
@NgModule({ imports: [SD, forwardRef(() => StandalonePipe), ReexportModule], exports: [forwardRef(() => SD), DirModule] })
export class AliasModule {}
",
            &[("AliasModule", Some("ReexportModule,DirModule"))],
            &cross_file(),
        );
    }

    #[test]
    fn package_declaration_files_are_read() {
        // ngtsc, with `@angular/common`: `[CommonModule, CommonModule]`.
        let dir = TempDir::new().unwrap();
        common_package(&dir);
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { NgIf, AsyncPipe, CommonModule, NgClass } from '@lib/common';
@NgModule({ imports: [NgIf, AsyncPipe, CommonModule], exports: [NgIf, CommonModule, NgClass] })
export class PkgModule {}
",
            &[("PkgModule", Some("CommonModule,CommonModule"))],
            &cross_file(),
        );
    }

    #[test]
    fn unresolved_imports_stay() {
        // A missing file and an `export *` that doesn't have the name; the
        // scope can't be read either, so the exported component stays.
        let dir = TempDir::new().unwrap();
        dep(&dir);
        write(&dir, "src/barrel.ts", "export * from './dep';\n");
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { Missing } from './missing';
import { NotThere } from './barrel';
import { StandaloneCmp } from './dep';
@NgModule({ imports: [Missing, NotThere], exports: [StandaloneCmp] })
export class M {}
",
            &[("M", Some("Missing,NotThere,StandaloneCmp"))],
            &cross_file(),
        );
    }

    #[test]
    fn an_exported_component_outside_a_readable_scope_stays() {
        // ngtsc rejects this module, but if the component's own imports give
        // providers, leaving it out would lose them: it stays.
        let dir = TempDir::new().unwrap();
        dep(&dir);
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { PlainModule, StandaloneCmp } from './dep';
@NgModule({ imports: [PlainModule], exports: [StandaloneCmp] })
export class M {}
",
            &[("M", Some("PlainModule,StandaloneCmp"))],
            &cross_file(),
        );
    }

    #[test]
    fn without_imported_values_imported_classes_stay() {
        let dir = TempDir::new().unwrap();
        dep(&dir);
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { StandaloneDir, DirModule } from './dep';
@NgModule({ imports: [StandaloneDir, DirModule] })
export class M {}
",
            &[("M", Some("StandaloneDir,DirModule"))],
            &TransformOptions::default(),
        );
    }

    #[test]
    fn files_read_are_dependencies() {
        let dir = TempDir::new().unwrap();
        dep(&dir);
        write(&dir, "src/barrel.ts", "export * from './dep';\n");
        let source = r"
import { NgModule } from '@angular/core';
import { StandaloneDir } from './barrel';
@NgModule({ imports: [StandaloneDir] })
export class M {}
";
        let path = write(&dir, "src/app.ts", source);
        let allocator = Allocator::default();
        let result = transform_angular_file(
            &allocator,
            path.to_str().unwrap(),
            source,
            Some(&cross_file()),
            None,
        );
        assert_eq!(injectors(&result.code), vec!["/*@__PURE__*/i0.ɵɵdefineInjector({})"]);
        let names: Vec<&str> = result
            .dependencies
            .iter()
            .filter_map(|dep| std::path::Path::new(dep).file_name()?.to_str())
            .collect();
        assert!(names.contains(&"barrel.ts"), "{:?}", result.dependencies);
        assert!(names.contains(&"dep.ts"), "{:?}", result.dependencies);
    }
}

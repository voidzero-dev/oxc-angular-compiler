//! `ɵinj` imports of an NgModule (#623). In full and partial compilation,
//! ngtsc's `NgModuleDecoratorHandler.resolve` leaves the directives and pipes
//! of `imports` out of the injector, and so a standalone component that
//! brings in no providers (`mayExportProviders`). An entry it keeps only part
//! of is written as the classes it keeps, one by one, and `...LIST` as `LIST`
//! when it keeps all of it. `exports` adds each NgModule by itself. Oxc does
//! the same when it knows what an entry is: from this file alone, or, with
//! cross-file analysis (`TransformOptions::resolve_imported_values`), from the
//! files and packages it imports. Anything else stays as written, and so does
//! an entry whose kept classes are read through `forwardRef`.
//!
//! Each expected output was checked against ngtsc 22.2.3. Where Oxc differs
//! on purpose, the test says so.

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

/// Each `ɵinj` of `code` as `(class, imports)`, whitespace removed, in
/// order; `None` when it has no `imports`. Other fields are left out.
fn module_imports(code: &str) -> Vec<(String, Option<String>)> {
    let compact: String = code.chars().filter(|c| !c.is_whitespace()).collect();
    let mut modules = Vec::new();
    let mut rest = compact.as_str();
    let mut before = 0;
    while let Some(at) = rest.find("ɵinj=") {
        let head = &compact[..before + at];
        let name = &head[head.rfind("class").unwrap() + 5..];
        let name = &name[..name.find(['{', '(']).unwrap()];
        let init = &rest[at..rest[at..].find(';').unwrap() + at];
        let imports = init.find("imports:[").map(|start| {
            let list = &init[start + 9..];
            let mut depth = 1;
            let end = list
                .char_indices()
                .find(|(_, c)| {
                    match c {
                        '[' => depth += 1,
                        ']' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .unwrap()
                .0;
            list[..end].to_string()
        });
        modules.push((name.to_string(), imports));
        before += at + 5;
        rest = &compact[before..];
    }
    modules
}

/// Compile `source` as `path` in full and partial compilation, and check
/// the `imports` of each `ɵinj`, by module, in order.
fn assert_module_imports(
    path: &str,
    source: &str,
    expected: &[(&str, Option<&str>)],
    base: &TransformOptions,
) -> [String; 2] {
    [CompilationMode::Full, CompilationMode::Partial].map(|mode| {
        let options = TransformOptions { compilation_mode: mode, ..base.clone() };
        let allocator = Allocator::default();
        let result = transform_angular_file(&allocator, path, source, Some(&options), None);
        assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
        let want: Vec<(String, Option<String>)> = expected
            .iter()
            .map(|(name, imports)| (name.to_string(), imports.map(Into::into)))
            .collect();
        assert_eq!(module_imports(&result.code), want, "{mode:?}");
        result.code
    })
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
    // ngtsc gives `{}` for both: `LocalCmp` is a standalone component that
    // brings in no providers.
    assert_injectors(
        r"
import { Directive, Pipe, NgModule, Component } from '@angular/core';
@Directive({ selector: '[sdir]' }) export class StandaloneDir {}
@Pipe({ name: 'spipe' }) export class StandalonePipe { transform(v: any) { return v; } }
@Component({ selector: 'lc', template: '' }) export class LocalCmp {}
@NgModule({ imports: [StandaloneDir, StandalonePipe], exports: [StandaloneDir, StandalonePipe] }) export class ReexportModule {}
@NgModule({ imports: [StandaloneDir, StandalonePipe, LocalCmp], exports: [StandaloneDir] }) export class LocalModule {}
",
        &[("ReexportModule", None), ("LocalModule", None)],
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
    // is an error in ngtsc; Oxc keeps it, as before.
    assert_injectors(
        r"
import { Component, NgModule } from '@angular/core';
@Component({ selector: 'c', template: '' }) export class Cmp {}
@Component({ selector: 'o', template: '' }) export class Orphan {}
@NgModule({ imports: [Cmp], exports: [Cmp, Orphan] }) export class M {}
",
        &[("M", Some("Orphan"))],
        &TransformOptions::default(),
    );
}

#[test]
fn a_component_that_is_not_standalone_stays() {
    // ngtsc fails here (-992011: not standalone, can't be imported). Oxc
    // keeps such a component, alone or in a list, as before. `Dir` is left
    // out.
    assert_injectors(
        r"
import { Component, Directive, NgModule } from '@angular/core';
@Component({ selector: 'l', template: '', standalone: false }) export class LocalNonStd {}
@Directive({ selector: '[d]' }) export class Dir {}
@NgModule({ imports: [LocalNonStd, [Dir, LocalNonStd]] }) export class M {}
",
        &[("M", Some("LocalNonStd,LocalNonStd"))],
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

/// The ngtsc case of standalone components with and without providers, in
/// one file.
const PROVIDER_CASES: &str = r"
import { Component, NgModule, Injectable, forwardRef } from '@angular/core';
@Injectable() export class Svc {}
@NgModule({ providers: [Svc] }) export class ProvModule {}
@NgModule({ providers: [] }) export class EmptyProvModule {}
@NgModule({}) export class PlainModule {}
@NgModule({ imports: [ProvModule] }) export class WrapProvModule {}
@NgModule({ imports: [PlainModule] }) export class WrapPlainModule {}
@Component({ selector: 'c-empty', template: '' }) export class CEmpty {}
@Component({ selector: 'c-own', template: '', providers: [Svc], viewProviders: [Svc] }) export class COwnProviders {}
@Component({ selector: 'c-prov', template: '', imports: [ProvModule] }) export class CProv {}
@Component({ selector: 'c-emptyprov', template: '', imports: [EmptyProvModule] }) export class CEmptyProv {}
@Component({ selector: 'c-plain', template: '', imports: [PlainModule] }) export class CPlain {}
@Component({ selector: 'c-wrapprov', template: '', imports: [WrapProvModule] }) export class CWrapProv {}
@Component({ selector: 'c-wrapplain', template: '', imports: [WrapPlainModule] }) export class CWrapPlain {}
@Component({ selector: 'c-nested-prov', template: '', imports: [CProv] }) export class CNestedProv {}
@Component({ selector: 'c-nested-empty', template: '', imports: [CEmpty] }) export class CNestedEmpty {}
@Component({ selector: 'c-cyc-a', template: '', imports: [forwardRef(() => CCycB)] }) export class CCycA {}
@Component({ selector: 'c-cyc-b', template: '', imports: [CCycA] }) export class CCycB {}
@Component({ selector: 'c-cyc-pa', template: '', imports: [forwardRef(() => CCycPB)] }) export class CCycPA {}
@Component({ selector: 'c-cyc-pb', template: '', imports: [CCycPA, ProvModule] }) export class CCycPB {}
@NgModule({ imports: [CEmpty, COwnProviders, CProv, CEmptyProv, CPlain, CWrapProv, CWrapPlain, CNestedProv, CNestedEmpty, CCycA, CCycB, CCycPA, CCycPB] })
export class AppModule {}
";

/// The modules of [`PROVIDER_CASES`] and their `ɵinj` imports, from ngtsc.
const PROVIDER_CASES_IMPORTS: &[(&str, Option<&str>)] = &[
    ("ProvModule", None),
    ("EmptyProvModule", None),
    ("PlainModule", None),
    ("WrapProvModule", Some("ProvModule")),
    ("WrapPlainModule", Some("PlainModule")),
    ("AppModule", Some("CProv,CEmptyProv,CWrapProv,CNestedProv,CCycPA,CCycPB")),
];

#[test]
fn standalone_components_without_providers_are_left_out() {
    // A component stays when its imports reach an NgModule with a
    // `providers` key (even `[]`), through components and NgModules. Its
    // own providers don't count.
    assert_module_imports(
        "app.ts",
        PROVIDER_CASES,
        PROVIDER_CASES_IMPORTS,
        &TransformOptions::default(),
    );
}

#[test]
fn any_providers_key_counts() {
    // ngtsc: `[CUndef, CConst]`; `CPipe` imports a pipe only.
    assert_module_imports(
        "app.ts",
        r"
import { Component, Pipe, NgModule } from '@angular/core';
@Pipe({ name: 'p' }) export class P { transform(v: any) { return v; } }
@Component({ selector: 'c-pipe', template: '', imports: [P] }) export class CPipe {}
@NgModule({ providers: undefined }) export class UndefProv {}
@Component({ selector: 'c-undef', template: '', imports: [UndefProv] }) export class CUndef {}
const PROVS: any[] = [];
@NgModule({ providers: PROVS }) export class ConstProv {}
@Component({ selector: 'c-const', template: '', imports: [ConstProv] }) export class CConst {}
@NgModule({ imports: [CPipe, CUndef, CConst] }) export class M {}
",
        &[("UndefProv", None), ("ConstProv", None), ("M", Some("CUndef,CConst"))],
        &TransformOptions::default(),
    );
}

/// Lists, `forwardRef` and `ModuleWithProviders` of one file, and a
/// component behind a `ModuleWithProviders`.
const SAME_FILE_LISTS: &str = r"
import { Component, Directive, Injectable, NgModule, forwardRef } from '@angular/core';
import { ImportedModule } from './other';
@Injectable() export class Svc {}
@Directive({ selector: '[ld]' }) export class LocalDir {}
@NgModule({}) export class LocalMod {}
@NgModule({ providers: [Svc] }) export class ProvModule {}
@NgModule({}) export class BareForRoot { static forRoot() { return { ngModule: BareForRoot, providers: [Svc] }; } }
@NgModule({ imports: [BareForRoot.forRoot()] }) export class ImportsMwpModule {}
@Component({ selector: 'c-mwp', template: '' , imports: [ImportsMwpModule] }) export class CMwp {}
@Component({ selector: 'c-imp', template: '', imports: [ImportedModule] }) export class CImported {}
const SAME = [LocalDir, LocalMod];
const SAME_IMP = [LocalDir, ImportedModule];
const EMPTY: any[] = [];
const SAME_FWD = [forwardRef(() => LocalDir), forwardRef(() => LateMod)];
const SAME_MWP = [LocalDir, BareForRoot.forRoot()];
@NgModule({ imports: [...SAME] }) export class M1 {}
@NgModule({ imports: [SAME_IMP] }) export class M2 {}
@NgModule({ imports: [[LocalDir, LocalMod], [LocalMod]] }) export class M3 {}
@NgModule({ imports: [...EMPTY, LocalMod] }) export class M4 {}
@NgModule({ imports: [...SAME_FWD] }) export class M5 {}
@NgModule({ imports: [forwardRef(() => LATER)] }) export class M6 {}
@NgModule({ imports: [...SAME_MWP] }) export class M7 {}
@NgModule({ imports: SAME }) export class M8 {}
@NgModule({ imports: [CMwp, CImported] }) export class M9 {}
@NgModule({}) export class LateMod {}
export const LATER = [LocalDir, LateMod];
";

/// The modules of [`SAME_FILE_LISTS`] and their `ɵinj` imports; `M2` and
/// `M9` as this file alone shows them.
const SAME_FILE_LISTS_IMPORTS: &[(&str, Option<&str>)] = &[
    ("LocalMod", None),
    ("ProvModule", None),
    ("BareForRoot", None),
    ("ImportsMwpModule", Some("BareForRoot.forRoot()")),
    ("M1", Some("LocalMod")),
    ("M2", Some("SAME_IMP")),
    ("M3", Some("LocalMod,[LocalMod]")),
    ("M4", Some("EMPTY,LocalMod")),
    // ngtsc: `[LateMod]`, which reads `LateMod` before it exists.
    ("M5", Some("...SAME_FWD")),
    ("M6", Some("forwardRef(()=>LATER)")),
    ("M7", Some("SAME_MWP")),
    ("M8", Some("LocalMod")),
    ("M9", Some("CImported")),
    ("LateMod", None),
];

#[test]
fn lists_of_this_file_are_written_like_ngtsc() {
    // A list kept in part is written as the classes it keeps, a spread kept
    // whole as its argument, and a list holding a `ModuleWithProviders` as
    // is. ngtsc also drops `CMwp`: it only reaches the providers of
    // `BareForRoot.forRoot()` through an NgModule without a `providers` key.
    // Without cross-file analysis, `ImportedModule` is not known, so `M2`
    // and `CImported` stay.
    assert_module_imports(
        "app.ts",
        SAME_FILE_LISTS,
        SAME_FILE_LISTS_IMPORTS,
        &TransformOptions::default(),
    );
}

#[test]
fn exports_of_this_file_are_written_one_by_one() {
    // ngtsc writes each exported NgModule by itself. For `E4` it writes
    // `[LateMod]`, which reads `LateMod` before it exists: the `forwardRef`
    // stays.
    assert_module_imports(
        "app.ts",
        r"
import { Directive, NgModule, forwardRef } from '@angular/core';
import { OtherModule as OM } from './dep';
@Directive({ selector: '[ld]', standalone: false }) export class LocalDir {}
@Directive({ selector: '[ld2]' }) export class StandaloneDir {}
@NgModule({}) export class LocalMod {}
const EMPTY: any[] = [];
const MODS = [LocalDir, LocalMod];
const MODS2 = [StandaloneDir, LocalMod];
@NgModule({ exports: [...EMPTY] }) export class E1 {}
@NgModule({ declarations: [LocalDir], exports: [...MODS] }) export class E2 {}
@NgModule({ imports: [StandaloneDir], exports: [...MODS2, OM] }) export class E3 {}
@NgModule({ exports: [forwardRef(() => LateMod)] }) export class E4 {}
@NgModule({}) export class LateMod {}
",
        &[
            ("LocalMod", None),
            ("E1", None),
            ("E2", Some("LocalMod")),
            ("E3", Some("LocalMod,OM")),
            ("E4", Some("forwardRef(()=>LateMod)")),
            ("LateMod", None),
        ],
        &TransformOptions::default(),
    );
}

#[test]
fn a_component_whose_providers_cannot_be_read_keeps_its_entry() {
    // ngtsc reads `mods()` and `FLAG`, finds no providers and leaves both
    // lists out (`ɵinj` without `imports`). Oxc can't read either, so the
    // lists stay as written rather than turning into `[UCmp]` / `[FCmp]`.
    assert_module_imports(
        "app.ts",
        r"
import { Component, Directive, NgModule } from '@angular/core';
const FLAG = true;
@Directive({ selector: '[d]' }) export class Dir {}
@NgModule({}) export class PlainModule {}
function mods() { return [PlainModule]; }
@Component({ selector: 'u', template: '', imports: [mods()] }) export class UCmp {}
@Component({ selector: 'f', template: '', standalone: FLAG }) export class FCmp {}
const L = [Dir, UCmp];
const LF = [Dir, FCmp];
@NgModule({ imports: [...L] }) export class M1 {}
@NgModule({ imports: [...LF] }) export class M2 {}
",
        &[("PlainModule", None), ("M1", Some("...L")), ("M2", Some("...LF"))],
        &TransformOptions::default(),
    );
}

#[cfg(feature = "cross_file_elision")]
mod cross_file {
    //! With cross-file analysis, imported classes, `const` lists and package
    //! declaration files are read too.

    use oxc_allocator::Allocator;
    use oxc_angular_compiler::{TransformOptions, transform_angular_file};
    use tempfile::TempDir;

    use super::{
        PROVIDER_CASES, PROVIDER_CASES_IMPORTS, SAME_FILE_LISTS, SAME_FILE_LISTS_IMPORTS,
        assert_injectors_at, assert_module_imports, injectors,
    };

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

    /// Compile `src/app.ts` in `dir` with cross-file analysis and check the
    /// `imports` of each `ɵinj`. Returns the full and the partial code.
    fn check_modules(
        dir: &TempDir,
        source: &str,
        expected: &[(&str, Option<&str>)],
    ) -> [String; 2] {
        let path = write(dir, "src/app.ts", source);
        assert_module_imports(path.to_str().unwrap(), source, expected, &cross_file())
    }

    /// `node_modules/fakelib`: a package's declaration file.
    fn fakelib(dir: &TempDir) {
        write(
            dir,
            "node_modules/fakelib/package.json",
            r#"{ "name": "fakelib", "types": "index.d.ts" }"#,
        );
        write(
            dir,
            "node_modules/fakelib/index.d.ts",
            r#"
import * as i0 from '@angular/core';
export declare class LibCmp {
  static ɵcmp: i0.ɵɵComponentDeclaration<LibCmp, "lib-cmp", never, {}, {}, never, never, true, never>;
}
export declare class LibDir {
  static ɵdir: i0.ɵɵDirectiveDeclaration<LibDir, "[libDir]", never, {}, {}, never, never, true, never>;
}
export declare class LibNonStdCmp {
  static ɵcmp: i0.ɵɵComponentDeclaration<LibNonStdCmp, "lib-ns", never, {}, {}, never, never, false, never>;
}
export declare class LibModule {
  static ɵmod: i0.ɵɵNgModuleDeclaration<LibModule, [typeof LibNonStdCmp], never, [typeof LibNonStdCmp]>;
  static ɵinj: i0.ɵɵInjectorDeclaration<LibModule>;
}
export declare const LIB_LIST: readonly [typeof LibDir, typeof LibModule];
"#,
        );
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
        // `StandaloneCmp` brings in no providers.
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
                        "DirModule,ReexportModule,LocalModule,DirModule,ReexportModule,LocalModule",
                    ),
                ),
            ],
            &cross_file(),
        );
    }

    #[test]
    fn namespaces_lists_and_reexported_components() {
        // A list kept in part is written as the classes it keeps, through a
        // new namespace import, and `StandaloneCmp` brings in no providers.
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
                ("MixedModule", Some("i1.PlainModule,i1.PlainModule")),
                ("CmpReexportModule", Some("CmpModule,CmpModule")),
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
    fn exports_stay_when_the_scope_cannot_be_read() {
        // Oxc can't read `decls()`, so it doesn't know whether `CmpA` is in
        // the module's scope: `...EXP` stays as written. ngtsc reads it and
        // writes `[SharedModule, i1.SharedModule]`.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Component, Directive, NgModule } from '@angular/core';
@Component({ selector: 'a', template: '', standalone: false }) export class CmpA {}
@Component({ selector: 'b', template: '', standalone: false }) export class CmpB {}
@Directive({ selector: '[b]', standalone: false }) export class DirB {}
@NgModule({}) export class SharedModule {}
export const DECLS = [CmpA, DirB];
export function decls() { return [CmpB]; }
export const EXP = [CmpA, DirB, SharedModule];
",
        );
        check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { DECLS, EXP, SharedModule, decls } from './dep';
@NgModule({ declarations: [...DECLS, decls()], imports: [SharedModule], exports: [...EXP] })
export class M {}
",
            &[("M", Some("SharedModule,...EXP"))],
        );
    }

    #[test]
    fn a_component_whose_providers_cannot_be_read_keeps_its_list() {
        // Oxc can't read `mods()`: `UCmp` may bring in providers, so `...L`
        // stays as written. ngtsc reads it and leaves the list out.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Component, Directive, NgModule } from '@angular/core';
@Directive({ selector: '[d]' }) export class Dir {}
@NgModule({}) export class PlainModule {}
export function mods() { return [PlainModule]; }
@Component({ selector: 'u', template: '', imports: [mods()] }) export class UCmp {}
export const L = [Dir, UCmp];
",
        );
        check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { L } from './dep';
@NgModule({ imports: [...L] }) export class M {}
",
            &[("M", Some("...L"))],
        );
    }

    #[test]
    fn a_package_class_reached_through_a_relative_file_keeps_its_entry() {
        // `./lib` re-exports the package, so the class has no package
        // specifier here. ngtsc writes `[i1.LibModule]` with a relative path
        // into node_modules, which may not resolve and, under a dev server,
        // loads a second copy of the package: the entry stays. Imported from
        // the package itself, the class is named through it.
        let dir = TempDir::new().unwrap();
        fakelib(&dir);
        write(&dir, "src/lib.ts", "export * from 'fakelib';\n");
        let [full, _] = check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { LIB_LIST } from './lib';
import { LIB_LIST as PKG_LIST } from 'fakelib';
@NgModule({ imports: [...LIB_LIST] }) export class M1 {}
@NgModule({ imports: [...PKG_LIST] }) export class M2 {}
",
            &[("M1", Some("...LIB_LIST")), ("M2", Some("i1.LibModule"))],
        );
        assert!(!full.contains("node_modules"), "{full}");
    }

    #[test]
    fn a_list_through_a_local_forward_ref_stays() {
        // This `forwardRef` returns `PlainModule`, not what its callback gives.
        let dir = TempDir::new().unwrap();
        dep(&dir);
        write(
            &dir,
            "src/list.ts",
            r"
import { PlainModule, StandaloneDir } from './dep';
function forwardRef(fn: () => unknown): any { return PlainModule; }
export const IMPORTS = [forwardRef(() => StandaloneDir)];
",
        );
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { IMPORTS } from './list';
@NgModule({ imports: [IMPORTS] })
export class M {}
",
            &[("M", Some("IMPORTS"))],
            &cross_file(),
        );
    }

    #[test]
    fn a_list_through_angular_forward_ref_is_left_out() {
        let dir = TempDir::new().unwrap();
        dep(&dir);
        write(
            &dir,
            "src/list.ts",
            r"
import { forwardRef as ref } from '@angular/core';
import { StandaloneDir } from './dep';
export const IMPORTS = [ref(() => StandaloneDir)];
",
        );
        check(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { IMPORTS } from './list';
@NgModule({ imports: [IMPORTS] })
export class M {}
",
            &[("M", None)],
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
    fn standalone_components_of_this_file_without_providers() {
        let dir = TempDir::new().unwrap();
        check_modules(&dir, PROVIDER_CASES, PROVIDER_CASES_IMPORTS);
    }

    #[test]
    fn lists_of_this_file_with_imported_classes() {
        // `ImportedModule` is now known: an NgModule without providers.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/other.ts",
            "import { NgModule } from '@angular/core';\n@NgModule({}) export class ImportedModule {}\n",
        );
        let expected: Vec<(&str, Option<&str>)> = SAME_FILE_LISTS_IMPORTS
            .iter()
            .map(|&(name, imports)| match name {
                "M2" => (name, Some("ImportedModule")),
                "M9" => (name, None),
                _ => (name, imports),
            })
            .collect();
        check_modules(&dir, SAME_FILE_LISTS, &expected);
    }

    #[test]
    fn imported_standalone_components() {
        // A component reaching providers through a `forwardRef`, a list, or
        // a package's NgModule stays. One with its own providers only, in a
        // cycle without providers, or with `imports: []` is left out.
        let dir = TempDir::new().unwrap();
        fakelib(&dir);
        write(
            &dir,
            "src/dep.ts",
            r"
import { Component, NgModule, Injectable, forwardRef } from '@angular/core';
import { LibModule } from 'fakelib';
@Injectable() export class Svc {}
@NgModule({ providers: [Svc] }) export class ProvModule {}
@NgModule({ providers: [] }) export class EmptyProvModule {}
@NgModule({}) export class PlainModule {}
const CLIST = [PlainModule, ProvModule];
@Component({ selector: 'a', template: '', imports: [forwardRef(() => ProvModule)] }) export class AFwdProv {}
@Component({ selector: 'b', template: '', imports: [...CLIST] }) export class BList {}
@Component({ selector: 'd', template: '', imports: [LibModule] }) export class DLibMod {}
@Component({ selector: 'e', template: '', imports: [EmptyProvModule] }) export class EEmptyProv {}
@Component({ selector: 'f', template: '', imports: [PlainModule], providers: [Svc] }) export class FPlain {}
@Component({ selector: 'g', template: '', imports: [forwardRef(() => HCyc)] }) export class GCyc {}
@Component({ selector: 'h', template: '', imports: [GCyc] }) export class HCyc {}
@Component({ selector: 'i', template: '', standalone: true, imports: [] }) export class IEmptyImports {}
",
        );
        let source = r"
import { NgModule } from '@angular/core';
import { AFwdProv, BList, DLibMod, EEmptyProv, FPlain, GCyc, HCyc, IEmptyImports } from './dep';
@NgModule({ imports: [AFwdProv, BList, DLibMod, EEmptyProv, FPlain, GCyc, HCyc, IEmptyImports] })
export class AppModule {}
";
        check_modules(&dir, source, &[("AppModule", Some("AFwdProv,BList,DLibMod,EEmptyProv"))]);
        let path = dir.path().join("src/app.ts");
        let allocator = Allocator::default();
        let result = transform_angular_file(
            &allocator,
            path.to_str().unwrap(),
            source,
            Some(&cross_file()),
            None,
        );
        assert!(
            result.dependencies.iter().any(|dep| dep.ends_with("dep.ts")),
            "{:?}",
            result.dependencies
        );
    }

    #[test]
    fn components_that_are_not_standalone_stay() {
        // ngtsc fails here (-992011: not standalone, can't be imported). Oxc
        // keeps such a component from another file, a package or a list, as
        // before. The directives are left out.
        let dir = TempDir::new().unwrap();
        dep(&dir);
        fakelib(&dir);
        write(
            &dir,
            "src/list.ts",
            "import { NonStdCmp } from './dep';\nexport const NL = [NonStdCmp];\n",
        );
        check_modules(
            &dir,
            r"
import { Component, Directive, NgModule } from '@angular/core';
import { NonStdCmp } from './dep';
import { NL } from './list';
import { LibNonStdCmp, LibDir } from 'fakelib';
@Component({ selector: 'l', template: '', standalone: false }) export class LocalNonStd {}
@Directive({ selector: '[d]' }) export class Dir {}
@NgModule({
  imports: [NonStdCmp, LibNonStdCmp, LocalNonStd, [Dir, LocalNonStd], [LibDir, LibNonStdCmp], ...NL],
})
export class M {}
",
            &[("M", Some("NonStdCmp,LibNonStdCmp,LocalNonStd,LocalNonStd,LibNonStdCmp,NL"))],
        );
    }

    #[test]
    fn a_component_with_imports_that_cannot_be_read_is_still_known() {
        // Oxc can't read `mods()`, so `UCmp` stays in the `ɵinj` (ngtsc
        // reads it and leaves `UCmp` out). `UCmp` is still a known
        // component: `ACmp` gets its scope, as before.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Component, Directive, NgModule } from '@angular/core';
@Directive({ selector: '[d]' }) export class Dir {}
@NgModule({}) export class PlainModule {}
export function mods() { return [PlainModule]; }
@Component({ selector: 'u', template: '', imports: [mods()] }) export class UCmp {}
",
        );
        let [_, partial] = check_modules(
            &dir,
            r"
import { Component, NgModule } from '@angular/core';
import { UCmp, Dir } from './dep';
@Component({ selector: 'a', template: '<u></u>', standalone: false }) export class ACmp {}
@NgModule({ declarations: [ACmp], imports: [UCmp, Dir] }) export class M {}
",
            &[("M", Some("UCmp"))],
        );
        assert!(
            partial
                .contains("i0.ɵɵsetComponentScope(ACmp, () => [i1.UCmp, i1.Dir, ACmp], () => []);"),
            "{partial}"
        );
    }

    #[test]
    fn module_with_providers_behind_a_component_is_not_read() {
        // Like ngtsc: an NgModule without a `providers` key brings in none,
        // even when it imports `BareForRoot.forRoot()`. Both components are
        // left out, so `Svc2` is not provided and `SideModule` is not
        // created with `AppModule`'s injector.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Component, NgModule, Injectable } from '@angular/core';
@Injectable() export class Svc2 {}
@NgModule({}) export class BareForRoot { static forRoot() { return { ngModule: BareForRoot, providers: [Svc2] }; } }
@NgModule({ imports: [BareForRoot.forRoot()] }) export class ImportsMwpModule {}
@Component({ selector: 'c', template: '', imports: [ImportsMwpModule] }) export class CMwp {}
@NgModule({}) export class SideModule { constructor() { (globalThis as any).ran = ((globalThis as any).ran || 0) + 1; } }
@Component({ selector: 'c-side', template: '', imports: [SideModule] }) export class CSide {}
",
        );
        check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { CMwp, CSide } from './dep';
@NgModule({ imports: [CMwp, CSide] }) export class AppModule {}
",
            &[("AppModule", None)],
        );
    }

    /// `src/dep.ts` and `src/local.d.ts` of the ngtsc case of lists from
    /// another file, with `node_modules/fakelib`.
    fn dep_lists(dir: &TempDir) {
        fakelib(dir);
        write(
            dir,
            "src/local.d.ts",
            r#"
import * as i0 from '@angular/core';
export declare class DtsCmp {
  static ɵcmp: i0.ɵɵComponentDeclaration<DtsCmp, "dts-cmp", never, {}, {}, never, never, true, never>;
}
"#,
        );
        write(
            dir,
            "src/dep.ts",
            r"
import { Component, Directive, NgModule, Injectable } from '@angular/core';
import { LibCmp, LibDir } from 'fakelib';
@Injectable() export class Svc {}
@NgModule({ providers: [Svc] }) export class ProvModule {}
@NgModule({}) export class PlainModule {}
@Directive({ selector: '[d]' }) export class Dir {}
@Component({ selector: 'x-empty', template: '' }) export class XEmpty {}
@Component({ selector: 'x-prov', template: '', imports: [ProvModule] }) export class XProv {}
@Component({ selector: 'x-libcmp', template: '', imports: [LibCmp] }) export class XLibCmp {}
@Component({ selector: 'x-libdir', template: '', imports: [LibDir] }) export class XLibDir {}
export const LIST = [Dir, ProvModule];
export const LIST_CMP = [XEmpty, XProv, PlainModule];
export const ALL_KEPT = [ProvModule, PlainModule];
export const ALL_DROP = [Dir, XEmpty];
",
        );
    }

    /// The app of the ngtsc case of lists from another file.
    const DEP_LISTS_APP: &str = r"
import { NgModule } from '@angular/core';
import { LibCmp, LibDir, LibModule, LIB_LIST } from 'fakelib';
import { DtsCmp } from './local';
import { XEmpty, XProv, XLibCmp, XLibDir, LIST, LIST_CMP, ALL_KEPT, ALL_DROP, Dir, ProvModule } from './dep';
import * as dep from './dep';
@NgModule({ imports: [XEmpty, XProv, XLibCmp, XLibDir, LibCmp, LibDir, LibModule, DtsCmp, dep.XEmpty, dep.XProv] })
export class AModule {}
@NgModule({ imports: [...LIST] }) export class SpreadModule {}
@NgModule({ imports: [LIST] }) export class NestedModule {}
@NgModule({ imports: LIST }) export class BareModule {}
@NgModule({ imports: [...LIST_CMP, Dir] }) export class SpreadCmpModule {}
@NgModule({ imports: [...ALL_KEPT, ALL_KEPT] }) export class AllKeptModule {}
@NgModule({ imports: [...ALL_DROP, ProvModule] }) export class AllDropModule {}
@NgModule({ imports: [...LIB_LIST] }) export class LibListModule {}
@NgModule({ imports: [dep.LIST] }) export class NsListModule {}
@NgModule({ imports: [...LIST, ProvModule] }) export class DupModule {}
";

    #[test]
    fn lists_from_another_file() {
        // A component of a declaration file is taken to bring in providers;
        // a list is written as the classes it keeps, named through a new
        // namespace import of the file or package they come from.
        let dir = TempDir::new().unwrap();
        dep_lists(&dir);
        let code = check_modules(
            &dir,
            DEP_LISTS_APP,
            &[
                ("AModule", Some("XProv,XLibCmp,LibCmp,LibModule,DtsCmp,dep.XProv")),
                ("SpreadModule", Some("i1.ProvModule")),
                ("NestedModule", Some("i1.ProvModule")),
                ("BareModule", Some("i1.ProvModule")),
                ("SpreadCmpModule", Some("i1.XProv,i1.PlainModule")),
                ("AllKeptModule", Some("ALL_KEPT,ALL_KEPT")),
                ("AllDropModule", Some("ProvModule")),
                ("LibListModule", Some("i2.LibModule")),
                ("NsListModule", Some("i1.ProvModule")),
                ("DupModule", Some("i1.ProvModule,ProvModule")),
            ],
        );
        for code in &code {
            assert!(code.contains("import * as i1 from './dep';"), "{code}");
            assert!(code.contains("import * as i2 from 'fakelib';"), "{code}");
        }
        // Like ngtsc, the JIT scope lists the classes too, the package's
        // tuple type read as a list: ngtsc full gives
        // `ɵɵsetNgModuleScope(SpreadModule, { imports: [i1.Dir, i1.ProvModule] })`
        // and `ɵɵsetNgModuleScope(LibListModule, { imports: [i2.LibDir, i2.LibModule] })`.
        let full: String = code[0].chars().filter(|c| !c.is_whitespace()).collect();
        assert!(
            full.contains("ɵɵsetNgModuleScope(SpreadModule,{imports:[i1.Dir,i1.ProvModule]})"),
            "{full}"
        );
        assert!(
            full.contains("ɵɵsetNgModuleScope(LibListModule,{imports:[i2.LibDir,i2.LibModule]})"),
            "{full}"
        );
    }

    #[test]
    fn lists_stay_without_cross_file() {
        let dir = TempDir::new().unwrap();
        dep_lists(&dir);
        let path = write(&dir, "src/app.ts", DEP_LISTS_APP);
        assert_module_imports(
            path.to_str().unwrap(),
            DEP_LISTS_APP,
            &[
                (
                    "AModule",
                    Some(
                        "XEmpty,XProv,XLibCmp,XLibDir,LibCmp,LibDir,LibModule,DtsCmp,dep.XEmpty,dep.XProv",
                    ),
                ),
                ("SpreadModule", Some("...LIST")),
                ("NestedModule", Some("LIST")),
                ("BareModule", Some("LIST")),
                ("SpreadCmpModule", Some("...LIST_CMP,Dir")),
                ("AllKeptModule", Some("...ALL_KEPT,ALL_KEPT")),
                ("AllDropModule", Some("...ALL_DROP,ProvModule")),
                ("LibListModule", Some("...LIB_LIST")),
                ("NsListModule", Some("dep.LIST")),
                ("DupModule", Some("...LIST,ProvModule")),
            ],
            &TransformOptions::default(),
        );
    }

    #[test]
    fn classes_are_named_from_the_file_that_declares_them() {
        // ngtsc imports `./third`, `fakelib`, `./dep` and `./local` in that
        // order. A list holding a `ModuleWithProviders` stays whole. For
        // `M10` and `M12` ngtsc writes the classes the `forwardRef`s give,
        // which can read them before they exist (`LateMod` does): Oxc keeps
        // those lists.
        let dir = TempDir::new().unwrap();
        fakelib(&dir);
        write(
            &dir,
            "src/third.ts",
            r"
import { Directive, NgModule } from '@angular/core';
@Directive({ selector: '[t]' }) export class ThirdDir {}
@NgModule({}) export class ThirdModule {}
",
        );
        write(
            &dir,
            "src/local.d.ts",
            r#"
import * as i0 from '@angular/core';
export declare class DtsDir {
  static ɵdir: i0.ɵɵDirectiveDeclaration<DtsDir, "[dtsd]", never, {}, {}, never, never, true, never>;
}
export declare class DtsModule {
  static ɵmod: i0.ɵɵNgModuleDeclaration<DtsModule, never, never, never>;
  static ɵinj: i0.ɵɵInjectorDeclaration<DtsModule>;
}
export declare const DL: readonly [typeof DtsDir, typeof DtsModule];
"#,
        );
        write(
            &dir,
            "src/dep.ts",
            r"
import { Directive, NgModule, forwardRef } from '@angular/core';
import { ThirdDir, ThirdModule } from './third';
import { LibDir, LibModule } from 'fakelib';
@Directive({ selector: '[d]' }) export class Dir {}
@NgModule({ providers: [] }) export class ForRootModule { static forRoot() { return { ngModule: ForRootModule, providers: [] }; } }
@NgModule({}) export default class DefModule {}
export const L3 = [ThirdDir, ThirdModule];
export const LLIB = [LibDir, LibModule];
export const LDEF = [Dir, DefModule];
export const LMWP = [Dir, ForRootModule.forRoot()];
export const LNEST = [[Dir], [ThirdModule, [LibModule]]];
export const LFWD = [forwardRef(() => Dir), forwardRef(() => ThirdModule)];
",
        );
        let code = check_modules(
            &dir,
            r"
import { Directive, NgModule, forwardRef } from '@angular/core';
import { Dir as D2, L3, LLIB, LDEF, LMWP, LNEST, LFWD } from './dep';
import { ThirdModule as TM } from './third';
import { DL } from './local';
@Directive({ selector: '[ld]' }) export class LocalDir {}
@NgModule({}) export class LocalMod {}
const SAME = [LocalDir, LocalMod];
const SAME_IMP = [D2, TM];
const EMPTY: any[] = [];
const SAME_FWD = [forwardRef(() => LocalDir), forwardRef(() => LateMod)];
@NgModule({ imports: [...SAME] }) export class M1 {}
@NgModule({ imports: [SAME_IMP] }) export class M2 {}
@NgModule({ imports: [[LocalDir, LocalMod], [LocalMod]] }) export class M3 {}
@NgModule({ imports: [...EMPTY, LocalMod] }) export class M4 {}
@NgModule({ imports: [...L3] }) export class M5 {}
@NgModule({ imports: [...LLIB] }) export class M6 {}
@NgModule({ imports: [...LDEF] }) export class M7 {}
@NgModule({ imports: [...LMWP] }) export class M8 {}
@NgModule({ imports: [LNEST] }) export class M9 {}
@NgModule({ imports: [...LFWD] }) export class M10 {}
@NgModule({ imports: [...DL] }) export class M11 {}
@NgModule({ imports: [...SAME_FWD] }) export class M12 {}
@NgModule({ imports: [...L3, TM, ...SAME] }) export class M13 {}
@NgModule({}) export class LateMod {}
",
            &[
                ("LocalMod", None),
                ("M1", Some("LocalMod")),
                ("M2", Some("TM")),
                ("M3", Some("LocalMod,[LocalMod]")),
                ("M4", Some("EMPTY,LocalMod")),
                ("M5", Some("i1.ThirdModule")),
                ("M6", Some("i2.LibModule")),
                ("M7", Some("i3.default")),
                ("M8", Some("LMWP")),
                ("M9", Some("i1.ThirdModule,i2.LibModule")),
                ("M10", Some("...LFWD")),
                ("M11", Some("i4.DtsModule")),
                ("M12", Some("...SAME_FWD")),
                ("M13", Some("i1.ThirdModule,TM,LocalMod")),
                ("LateMod", None),
            ],
        );
        for code in code {
            assert!(code.contains("import * as i1 from './third';"), "{code}");
            assert!(code.contains("import * as i2 from 'fakelib';"), "{code}");
            assert!(code.contains("import * as i3 from './dep';"), "{code}");
            assert!(code.contains("import * as i4 from './local';"), "{code}");
        }
    }

    /// `src/third.ts`: a directive and an NgModule.
    fn third(dir: &TempDir) {
        write(
            dir,
            "src/third.ts",
            r"
import { Directive, NgModule } from '@angular/core';
@Directive({ selector: '[t]' }) export class ThirdDir {}
@NgModule({}) export class ThirdModule {}
",
        );
    }

    #[test]
    fn namespace_members_get_their_own_namespace() {
        // ngtsc doesn't reuse `third`: it imports `./third` again as `i1`.
        let dir = TempDir::new().unwrap();
        third(&dir);
        write(
            &dir,
            "src/dep.ts",
            "import { ThirdDir, ThirdModule } from './third';\nexport const L3 = [ThirdDir, ThirdModule];\n",
        );
        let code = check_modules(
            &dir,
            r"
import { Directive, NgModule } from '@angular/core';
import * as third from './third';
import * as dep from './dep';
import { ThirdModule as TM } from './third';
@Directive({ selector: '[ld]' }) export class LocalDir {}
const NS = [third.ThirdDir, third.ThirdModule];
@NgModule({ imports: [...NS] }) export class N1 {}
@NgModule({ imports: [...dep.L3] }) export class N2 {}
@NgModule({ imports: [LocalDir, ...NS, third.ThirdModule] }) export class N3 {}
@NgModule({ imports: [[third.ThirdDir, third.ThirdModule]] }) export class N4 {}
@NgModule({ imports: [[LocalDir, TM]] }) export class N5 {}
",
            &[
                ("N1", Some("i1.ThirdModule")),
                ("N2", Some("i1.ThirdModule")),
                ("N3", Some("i1.ThirdModule,third.ThirdModule")),
                ("N4", Some("i1.ThirdModule")),
                ("N5", Some("TM")),
            ],
        );
        for code in code {
            assert!(code.contains("import * as i1 from './third';"), "{code}");
        }
    }

    #[test]
    fn exports_are_written_one_by_one() {
        let dir = TempDir::new().unwrap();
        third(&dir);
        write(
            &dir,
            "src/dep.ts",
            r"
import { Directive, NgModule, Injectable } from '@angular/core';
import { ThirdDir, ThirdModule } from './third';
@Injectable() export class Svc {}
@NgModule({}) export class DepModule { static forRoot() { return { ngModule: DepModule, providers: [Svc] }; } }
@NgModule({}) export class OtherModule {}
@Directive({ selector: '[d]' }) export class DepDir {}
export const MODS = [DepModule, OtherModule];
export const L3 = [ThirdDir, ThirdModule];
",
        );
        check_modules(
            &dir,
            r"
import { Directive, NgModule } from '@angular/core';
import * as dep from './dep';
import * as third from './third';
import { MODS, DepDir, L3, OtherModule as OM } from './dep';
import { ThirdModule } from './third';
@Directive({ selector: '[ld]', standalone: false }) export class LocalDir {}
@NgModule({}) export class LocalMod {}
const MIX = [LocalMod, OM];
const ALLMOD = [LocalMod, ThirdModule];
@NgModule({ exports: [dep.OtherModule] }) export class E1 {}
@NgModule({ exports: MODS }) export class E2 {}
@NgModule({ exports: [...MIX, OM] }) export class E3 {}
@NgModule({ declarations: [LocalDir], exports: [LocalDir, ...MODS] }) export class E4 {}
@NgModule({ imports: [DepDir], exports: [DepDir, LocalMod] }) export class E5 {}
@NgModule({ imports: [...ALLMOD], exports: [...ALLMOD] }) export class E6 {}
@NgModule({ imports: [...L3], exports: [...L3] }) export class E7 {}
@NgModule({ imports: [third.ThirdModule], exports: [third.ThirdModule] }) export class E8 {}
",
            &[
                ("LocalMod", None),
                ("E1", Some("i1.OtherModule")),
                ("E2", Some("i1.DepModule,i1.OtherModule")),
                ("E3", Some("LocalMod,OM,OM")),
                ("E4", Some("i1.DepModule,i1.OtherModule")),
                ("E5", Some("LocalMod")),
                ("E6", Some("ALLMOD,LocalMod,ThirdModule")),
                ("E7", Some("i2.ThirdModule,i2.ThirdModule")),
                ("E8", Some("third.ThirdModule,i2.ThirdModule")),
            ],
        );
    }

    #[test]
    fn duplicates_chains_and_component_lists() {
        // Duplicates are kept; `CmpModule` drops `XViaEmpty`, which imports
        // a component without providers. For `FwdListModule` ngtsc writes
        // `[LateMod]`, which reads `LateMod` before it exists: the
        // `forwardRef` stays.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Directive, NgModule, Injectable, Component } from '@angular/core';
@Injectable() export class Svc {}
@NgModule({ providers: [Svc] }) export class ProvModule {}
@NgModule({}) export class PlainModule {}
@Directive({ selector: '[d]' }) export class Dir {}
@Component({ selector: 'x-empty', template: '' }) export class XEmpty {}
export const DUP = [ProvModule, Dir, ProvModule];
export const INNER = [Dir, PlainModule];
export const CHAIN = [...INNER, ProvModule];
export const KEPT = [ProvModule, PlainModule];
export const CMP_LIST = [ProvModule];
@Component({ selector: 'x-list', template: '', imports: CMP_LIST }) export class XListImports {}
@Component({ selector: 'x-fwd', template: '', imports: [XEmpty] }) export class XViaEmpty {}
",
        );
        check_modules(
            &dir,
            r"
import { NgModule, Directive, forwardRef } from '@angular/core';
import { DUP, CHAIN, KEPT, XListImports, XViaEmpty } from './dep';
@Directive({ selector: '[ld]' }) export class LocalDir {}
@NgModule({ imports: [...DUP] }) export class DupModule {}
@NgModule({ imports: [...CHAIN] }) export class ChainModule {}
@NgModule({ imports: KEPT }) export class BareKeptModule {}
@NgModule({ imports: [XListImports, XViaEmpty] }) export class CmpModule {}
@NgModule({ imports: [forwardRef(() => LATER)] }) export class FwdListModule {}
@NgModule({}) export class LateMod {}
export const LATER = [LocalDir, LateMod];
",
            &[
                ("DupModule", Some("i1.ProvModule,i1.ProvModule")),
                ("ChainModule", Some("i1.PlainModule,i1.ProvModule")),
                ("BareKeptModule", Some("KEPT")),
                ("CmpModule", Some("XListImports")),
                ("FwdListModule", Some("forwardRef(()=>LATER)")),
                ("LateMod", None),
            ],
        );
    }

    #[test]
    fn module_with_providers_lists_stay_whole() {
        // A list holding a `ModuleWithProviders`, in any form, is written as
        // is: listing its NgModule alone would lose the providers.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Directive, Injectable, NgModule } from '@angular/core';
@Injectable() export class Svc {}
@NgModule({}) export class RootModule { static forRoot() { return { ngModule: RootModule, providers: [Svc] }; } }
@Directive({ selector: '[d]' }) export class Dir {}
export const ROOT = RootModule.forRoot();
export const L_CALL = [Dir, RootModule.forRoot()];
export const L_LIT = [Dir, { ngModule: RootModule, providers: [Svc] }];
export const L_CONST = [Dir, ROOT];
export const L_NEST = [Dir, [L_CALL]];
",
        );
        check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { L_CALL, L_LIT, L_CONST, L_NEST } from './dep';
@NgModule({ imports: [...L_CALL] }) export class CallModule {}
@NgModule({ imports: [...L_LIT] }) export class LitModule {}
@NgModule({ imports: [L_CONST] }) export class ConstModule {}
@NgModule({ imports: [...L_NEST] }) export class NestModule {}
",
            &[
                ("CallModule", Some("L_CALL")),
                ("LitModule", Some("L_LIT")),
                ("ConstModule", Some("L_CONST")),
                ("NestModule", Some("L_NEST")),
            ],
        );
    }

    #[test]
    fn a_class_that_cannot_be_imported_keeps_its_list() {
        // `Hidden` is not exported: ngtsc fails ("Unable to import class
        // Hidden"). The list stays as written.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Directive, NgModule, Injectable } from '@angular/core';
@Injectable() export class Svc {}
@NgModule({ providers: [Svc] }) class Hidden {}
@Directive({ selector: '[d]' }) export class Dir {}
export const HID = [Dir, Hidden];
",
        );
        check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { HID } from './dep';
@NgModule({ imports: [...HID] }) export class HidModule {}
",
            &[("HidModule", Some("...HID"))],
        );
    }

    #[test]
    fn namespace_names_avoid_the_file_bindings() {
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/dep.ts",
            r"
import { Directive, NgModule } from '@angular/core';
@NgModule({}) export class PlainModule {}
@Directive({ selector: '[d]' }) export class Dir {}
export const LIST = [Dir, PlainModule];
",
        );
        let code = check_modules(
            &dir,
            r"
import { NgModule } from '@angular/core';
import { LIST } from './dep';
export const i1 = 'user';
@NgModule({ imports: [...LIST] }) export class M {}
",
            &[("M", Some("i1_1.PlainModule"))],
        );
        for code in code {
            assert!(code.contains("import * as i1_1 from './dep';"), "{code}");
        }
    }

    #[test]
    fn a_library_class_is_named_through_its_entry_point() {
        // `@ws/lib` is a workspace library behind a tsconfig path. ngtsc
        // names `InnerModule` through it, and fails ("Unable to import class
        // InnerModule") when the entry point doesn't export it: the list
        // stays as written then.
        for (exports, expected) in
            [("LIST, Dir, InnerModule", "i1.InnerModule"), ("LIST, Dir", "...LIST")]
        {
            let dir = TempDir::new().unwrap();
            write(
                &dir,
                "libs/lib/inner.ts",
                r"
import { Directive, NgModule, Injectable } from '@angular/core';
@Injectable() export class Svc {}
@NgModule({ providers: [Svc] }) export class InnerModule {}
@Directive({ selector: '[d]' }) export class Dir {}
export const LIST = [Dir, InnerModule];
",
            );
            write(&dir, "libs/lib/index.ts", &format!("export {{ {exports} }} from './inner';\n"));
            let tsconfig = write(
                &dir,
                "tsconfig.json",
                r#"{ "compilerOptions": { "paths": { "@ws/lib": ["./libs/lib/index.ts"] } } }"#,
            );
            let source = r"
import { NgModule } from '@angular/core';
import { LIST } from '@ws/lib';
@NgModule({ imports: [...LIST] }) export class M {}
";
            let path = write(&dir, "src/app.ts", source);
            let options = TransformOptions { tsconfig_path: Some(tsconfig), ..cross_file() };
            assert_module_imports(
                path.to_str().unwrap(),
                source,
                &[("M", Some(expected))],
                &options,
            );
        }
    }

    #[test]
    fn files_a_component_imports_are_dependencies() {
        // Whether `Cmp` stays depends on `./plain`, which `app.ts` doesn't
        // import.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "src/plain.ts",
            "import { NgModule } from '@angular/core';\n@NgModule({}) export class PlainModule {}\n",
        );
        write(
            &dir,
            "src/cmp.ts",
            r"
import { Component } from '@angular/core';
import { PlainModule } from './plain';
@Component({ selector: 'c', template: '', imports: [PlainModule] }) export class Cmp {}
",
        );
        let source = r"
import { NgModule } from '@angular/core';
import { Cmp } from './cmp';
@NgModule({ imports: [Cmp] }) export class M {}
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
        assert!(
            result.dependencies.iter().any(|dep| dep.ends_with("plain.ts")),
            "{:?}",
            result.dependencies
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

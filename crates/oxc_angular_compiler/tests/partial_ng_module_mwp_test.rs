//! The lists of a partial `ɵɵngDeclareNgModule` (#623). ngtsc lists what
//! each entry gives (`resolveTypeList`): `imports: [mwp, Holder.forRoot()]`
//! becomes `imports: [DepModule, DepModule]`, and spreads, nested arrays,
//! `const` lists and namespace members are flattened. The injector and the
//! full-mode `ɵɵsetNgModuleScope` keep the entries as written. In both
//! modes the lists are wrapped in `() => [...]` when one names a class this
//! file declares after the NgModule.
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
    // - `later`: the module is declared after AppModule, so ngtsc lists it
    //   and wraps the lists in `() => [...]`.
    // - `typeOnly`: `DepModule` is only a type here, so its import is
    //   removed; ngtsc names it anyway and drops the import. Oxc keeps it
    //   as before.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
import { DepModule } from './dep';
export class Holder {
  static later(): ModuleWithProviders<LaterModule> { const p: any[] = []; return { ngModule: LaterModule, providers: p }; }
  static typeOnly(): ModuleWithProviders<DepModule> { const p: any[] = []; return { ngModule: null as any, providers: p }; }
}
@NgModule({ imports: [Holder.later(), Holder.typeOnly()] })
export class AppModule {}
@NgModule({})
export class LaterModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:()=>[LaterModule,Holder]}"), "{code}");
    assert!(!code.contains("'./dep'"), "the type-only import stays removed:\n{code}");
}

#[test]
fn a_parameter_or_type_parameter_keeps_its_old_output() {
    // The module comes from the call's argument or type argument, which
    // oxc doesn't follow.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders, Type } from '@angular/core';
import * as m from './dep';
@NgModule({})
export class AModule {}
export class Holder {
  static of(ngModule: Type<any>) { return { ngModule, providers: [] }; }
  static generic<T>(): ModuleWithProviders<T> { const m: any = null; return m; }
  static member(m: { DepModule: Type<any> }) { return { ngModule: m.DepModule }; }
}
@NgModule({ imports: [Holder.of(AModule), Holder.generic<AModule>(), Holder.member({ DepModule: AModule })] })
export class AppModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:[Holder,Holder,Holder]}"), "{code}");
}

#[test]
fn overloads_read_the_first_declaration() {
    // ngtsc partial: `imports: [AMod]`, whatever the arguments or body.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({}) export class AMod {}
@NgModule({}) export class BMod {}
export class Holder {
  static forRoot(): ModuleWithProviders<AMod>;
  static forRoot(x: number): ModuleWithProviders<BMod>;
  static forRoot(x?: number): any { return { ngModule: BMod, providers: [] }; }
}
@NgModule({ imports: [Holder.forRoot(1)] })
export class M {}
",
    );
    assert!(code.contains("type:M,imports:[AMod]}"), "{code}");
}

/// Holders whose single `return` ngtsc reads through, follows, or gives up
/// on (then reading the `ModuleWithProviders<T>` return type).
const SHAPES: &str = r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({}) export class DepModule {}
@NgModule({}) export class EmptyModule {}
@NgModule({}) export class OtherModule {}
export class Inner { static forRoot(): ModuleWithProviders<OtherModule> { return { ngModule: OtherModule }; } }
export class NoType { static forRoot(): any { const p: any[] = []; return { ngModule: OtherModule, providers: p }; } }
export class H {
  static sat(): ModuleWithProviders<DepModule> { return { ngModule: EmptyModule } satisfies ModuleWithProviders<EmptyModule>; }
  static cast(): ModuleWithProviders<DepModule> { return <ModuleWithProviders<EmptyModule>>{ ngModule: EmptyModule }; }
  static getter(): ModuleWithProviders<DepModule> { return { ngModule: EmptyModule as any, get providers() { return []; } }; }
  static method(): ModuleWithProviders<DepModule> { return { ngModule: EmptyModule as any, foo() {} } as any; }
  static paren(): ModuleWithProviders<DepModule> { return ({ ngModule: EmptyModule } as any)!; }
  static wrapTyped(): ModuleWithProviders<DepModule> { return Inner.forRoot(); }
  static wrapUntyped() { return Inner.forRoot(); }
  static wrapDynamic(): ModuleWithProviders<DepModule> { return NoType.forRoot(); }
}
";

#[test]
fn a_returned_call_is_followed_and_an_unreadable_body_reads_the_type() {
    // ngtsc partial, entry by entry: `satisfies`, `<T>x`, a getter and a
    // method give up on the body (`DepModule`); `()`, `as` and `!` are read
    // through (`EmptyModule`); a returned call is evaluated, also over the
    // return type (`OtherModule`), and falls back to it when ngtsc can't
    // evaluate the call (`DepModule`).
    let code = partial(&format!(
        "{SHAPES}@NgModule({{ imports: [H.sat(), H.cast(), H.getter(), H.method(), H.paren(), H.wrapTyped(), H.wrapUntyped(), H.wrapDynamic()] }})\nexport class AppModule {{}}\n"
    ));
    assert!(
        code.contains(
            "type:AppModule,imports:[DepModule,DepModule,DepModule,DepModule,EmptyModule,OtherModule,OtherModule,DepModule]}"
        ),
        "{code}"
    );
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

fn full(source: &str) -> String {
    compile_in("app.ts", source, &TransformOptions::default())
}

/// `Holder.forRoot()` gives `LaterModule`, declared after AppModule.
const LATER: &str = r"
import { NgModule, ModuleWithProviders, Component } from '@angular/core';
@Component({ selector: 'app-c', template: 'x', standalone: false })
export class AppCmp {}
export class Holder {
  static forRoot(): ModuleWithProviders<LaterModule> { return { ngModule: LaterModule, providers: [] }; }
}
@NgModule({})
export class EarlyModule {}
@NgModule({ declarations: [AppCmp], imports: [EarlyModule, Holder.forRoot()], exports: [AppCmp], bootstrap: [AppCmp] })
export class AppModule {}
@NgModule({})
export class LaterModule {}
";

#[test]
fn a_module_declared_later_wraps_every_list() {
    // ngtsc wraps all four lists once one names a class declared after the
    // NgModule, which isn't initialized yet when `ɵmod` is created.
    let code = partial(LATER);
    assert!(
        code.contains(
            "type:AppModule,bootstrap:()=>[AppCmp],declarations:()=>[AppCmp],imports:()=>[EarlyModule,LaterModule],exports:()=>[AppCmp]}"
        ),
        "{code}"
    );
    assert!(code.contains("type:AppModule,imports:[EarlyModule,Holder.forRoot()]}"), "{code}");
}

#[test]
fn a_return_type_naming_a_later_module_wraps_the_list() {
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
export class Holder {
  static typed(): ModuleWithProviders<LaterModule> { const p: any[] = []; return { ngModule: null as any, providers: p }; }
}
@NgModule({ imports: [Holder.typed()] })
export class AppModule {}
@NgModule({})
export class LaterModule {}
",
    );
    assert!(code.contains("type:AppModule,imports:()=>[LaterModule]}"), "{code}");
}

#[test]
fn full_mode_wraps_lists_like_ngtsc_and_keeps_raw_ones() {
    // ngtsc full: `bootstrap: () => [AppCmp]` and
    // `{ declarations: () => [AppCmp], imports: () => [EarlyModule, LaterModule], exports: () => [AppCmp] }`.
    // The raw `imports` stays as written and unwrapped: the runtime only
    // flattens and unwraps a list that is an array (`convertToTypeArray`).
    let code = full(LATER);
    assert!(code.contains("ɵɵdefineNgModule({type:AppModule,bootstrap:()=>[AppCmp]})"), "{code}");
    assert!(
        code.contains(
            "ɵɵsetNgModuleScope(AppModule,{declarations:()=>[AppCmp],imports:[EarlyModule,Holder.forRoot()],exports:()=>[AppCmp]})"
        ),
        "{code}"
    );
}

/// `forwardRef` to classes declared before the NgModule, and to an import.
const FORWARD_REFS: &str = r"
import { NgModule, Component, forwardRef } from '@angular/core';
import { DepModule } from './dep';
@NgModule({})
export class EarlyModule {}
@Component({ selector: 'b', template: '', standalone: false })
export class BCmp {}
@NgModule({ declarations: [BCmp], imports: [forwardRef(() => EarlyModule)], exports: [forwardRef(() => BCmp)] })
export class AllEarly {}
@NgModule({ imports: [forwardRef(() => DepModule)] })
export class ViaImport {}
";

#[test]
fn a_forward_ref_to_an_earlier_class_is_not_wrapped() {
    // ngtsc wraps by where the class is declared, not by `forwardRef`. An
    // import may be in a cycle with this file, so without cross-file
    // analysis its wrap stays (ngtsc: `imports: [DepModule]`).
    let code = partial(FORWARD_REFS);
    assert!(
        code.contains("type:AllEarly,declarations:[BCmp],imports:[EarlyModule],exports:[BCmp]}"),
        "{code}"
    );
    assert!(code.contains("type:ViaImport,imports:()=>[DepModule]}"), "{code}");
    let code = full(FORWARD_REFS);
    assert!(
        code.contains(
            "ɵɵsetNgModuleScope(AllEarly,{declarations:[BCmp],imports:[EarlyModule],exports:[BCmp]})"
        ),
        "{code}"
    );
    assert!(code.contains("ɵɵsetNgModuleScope(ViaImport,{imports:()=>[DepModule]})"), "{code}");
}

#[test]
fn spreads_nested_arrays_and_const_lists_are_flattened() {
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class AModule {
  static forRoot(): ModuleWithProviders<AModule> { return { ngModule: AModule, providers: [] }; }
}
@NgModule({})
export class BModule {}
export class Holder {
  static forRoot(): ModuleWithProviders<BModule> { return { ngModule: BModule, providers: [] }; }
}
const mwp = Holder.forRoot();
const MODS = [AModule.forRoot(), BModule, mwp];
const NESTED = [MODS, [Holder.forRoot()]];
@NgModule({ imports: [...MODS, AModule] })
export class SpreadModule {}
@NgModule({ imports: [[Holder.forRoot(), AModule], BModule] })
export class NestedModule {}
@NgModule({ imports: MODS })
export class BareModule {}
@NgModule({ imports: [NESTED, ...[mwp]] })
export class DeepModule {}
@NgModule({ imports: [AModule], exports: [...(MODS as any[])] })
export class ExportSpreadModule {}
",
    );
    assert!(
        code.contains("type:SpreadModule,imports:[AModule,BModule,BModule,AModule]}"),
        "{code}"
    );
    assert!(code.contains("type:NestedModule,imports:[BModule,AModule,BModule]}"), "{code}");
    assert!(code.contains("type:BareModule,imports:[AModule,BModule,BModule]}"), "{code}");
    assert!(
        code.contains("type:DeepModule,imports:[AModule,BModule,BModule,BModule,BModule]}"),
        "{code}"
    );
    assert!(
        code.contains(
            "type:ExportSpreadModule,imports:[AModule],exports:[AModule,BModule,BModule]}"
        ),
        "{code}"
    );
    assert!(
        code.contains("type:SpreadModule,imports:[...MODS,AModule]}"),
        "ɵinj stays raw:\n{code}"
    );
}

#[test]
fn a_later_class_in_a_const_list_wraps_the_lists() {
    // ngtsc partial, for each module: `imports: () => [EarlyModule,
    // LaterModule]` (through a `ModuleWithProviders` and a `forwardRef` in a
    // list), `imports: [EarlyModule, EarlyModule]` (an alias), and
    // `declarations: () => [ACmp, LaterCmp], exports: () => [ACmp, LaterCmp]`.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders, Component, forwardRef } from '@angular/core';
export class Holder {
  static forRoot(): ModuleWithProviders<LaterModule> { return { ngModule: LaterModule, providers: [] }; }
}
@NgModule({})
export class EarlyModule {}
@Component({ selector: 'a', template: '', standalone: false })
export class ACmp {}
const VIA_MWP = [EarlyModule, Holder.forRoot()];
const VIA_FORWARD_REF = [EarlyModule, forwardRef(() => LaterModule)];
const ALIAS = EarlyModule;
const DECLS = [ACmp, forwardRef(() => LaterCmp)];
@NgModule({ imports: [...VIA_MWP] })
export class MwpModule {}
@NgModule({ imports: [...VIA_FORWARD_REF] })
export class ForwardRefModule {}
@NgModule({ imports: [ALIAS, ...[ALIAS]] })
export class AliasModule {}
@NgModule({ declarations: [...DECLS], exports: [...DECLS] })
export class DeclModule {}
@NgModule({})
export class LaterModule {}
@Component({ selector: 'l', template: '', standalone: false })
export class LaterCmp {}
",
    );
    assert!(code.contains("type:MwpModule,imports:()=>[EarlyModule,LaterModule]}"), "{code}");
    assert!(
        code.contains("type:ForwardRefModule,imports:()=>[EarlyModule,LaterModule]}"),
        "{code}"
    );
    assert!(code.contains("type:AliasModule,imports:[EarlyModule,EarlyModule]}"), "{code}");
    assert!(
        code.contains(
            "type:DeclModule,declarations:()=>[ACmp,LaterCmp],exports:()=>[ACmp,LaterCmp]}"
        ),
        "{code}"
    );
}

#[test]
fn an_unread_entry_keeps_its_name_next_to_a_later_class() {
    // ngtsc partial: `imports: () => [Unknown, LaterModule, LaterModule]`.
    // Without cross-file analysis `Unknown` keeps its old output.
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
import { Unknown } from './dep';
export class Holder {
  static typed(): ModuleWithProviders<LaterModule> { const p: any[] = []; return { ngModule: null as any, providers: p }; }
}
const L = [Holder.typed()];
@NgModule({ imports: [Unknown, ...L, ...(L as any[])] })
export class AppModule {}
@NgModule({})
export class LaterModule {}
",
    );
    assert!(
        code.contains("type:AppModule,imports:()=>[Unknown,LaterModule,LaterModule]}"),
        "{code}"
    );
}

#[test]
fn lists_oxc_cant_read_keep_their_old_output() {
    // - an imported name inside a list, or a namespace member, without
    //   cross-file analysis (ngtsc: `[DepModule, i1.DepModule]`);
    // - a spread of a value that is no array, and `satisfies`, which ngtsc
    //   rejects (error -991010);
    // - a `let` list, a function call and a condition, which ngtsc reads
    //   (`[AModule, AModule, AModule]`).
    let code = partial(
        r"
import { NgModule, ModuleWithProviders } from '@angular/core';
import { DepModule } from './dep';
import * as ns from './dep';
@NgModule({})
export class AModule {
  static forRoot(): ModuleWithProviders<AModule> { return { ngModule: AModule, providers: [] }; }
}
const LOCAL = [DepModule];
const ONE = AModule.forRoot();
let LET_LIST = [AModule];
function make() { return AModule; }
function cond() { return true; }
@NgModule({ imports: [...LOCAL, ns.DepModule] })
export class ImportedModule {}
@NgModule({ imports: [...(ONE as any)] })
export class OneModule {}
@NgModule({ imports: [...([AModule] satisfies any[])] })
export class SatisfiesModule {}
@NgModule({ imports: [...LET_LIST, [make()], ...(cond() ? [AModule] : [])] })
export class EvaluatedModule {}
",
    );
    for module in ["ImportedModule", "OneModule", "SatisfiesModule", "EvaluatedModule"] {
        assert!(code.contains(&format!("type:{module}}})")), "{module}:\n{code}");
    }
}

#[test]
fn a_spread_in_a_class_in_a_function_keeps_its_old_output() {
    let code = partial(
        r"
import { NgModule } from '@angular/core';
@NgModule({})
export class DepModule {}
const MODS = [DepModule];
export function make() {
  @NgModule({ imports: [...MODS] })
  class AppModule {}
  return AppModule;
}
",
    );
    assert!(code.contains("type:AppModule})"), "{code}");
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
    fn a_module_declared_later_in_this_file_wraps_the_list() {
        // Another file's `ModuleWithProviders<Later>` names a class this file
        // declares after the NgModule. Like ngtsc, it is listed in
        // `() => [...]`, so it is read once it is initialized.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "holder.ts",
            r"
import { ModuleWithProviders } from '@angular/core';
import type { Later } from './app';
export class Holder {
  static forRoot(): ModuleWithProviders<Later> {
    const providers: any[] = [];
    return { ngModule: null as any, providers };
  }
}
export const LATER_ROOT = Holder.forRoot();
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { Holder, LATER_ROOT } from './holder';
@NgModule({ imports: [Holder.forRoot(), LATER_ROOT] })
export class M {}
@NgModule({})
export class Later {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("type:M,imports:()=>[Later,Later]}"), "{code}");
    }

    #[test]
    fn a_member_of_a_parameter_keeps_its_old_output() {
        // `modules.DepModule` reads the parameter, not the namespace import of
        // the same name.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP);
        write(
            &dir,
            "holder.ts",
            r"
import { Type } from '@angular/core';
import * as modules from './dep';
export class Holder {
  static forRoot(modules: { DepModule: Type<any> }) { return { ngModule: modules.DepModule }; }
}
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { Holder } from './holder';
@NgModule({ imports: [Holder.forRoot({ DepModule: Holder })] })
export class M {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("type:M,imports:[Holder]}"), "{code}");
    }

    #[test]
    fn imported_overloads_read_the_first_declaration() {
        // ngtsc partial: `imports: [i1.AMod, i1.AMod]`, for a source file
        // and for a declaration file alike.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "lib.ts",
            r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({}) export class AMod {}
@NgModule({}) export class BMod {}
@NgModule({}) export class CMod {}
export class Holder {
  static forRoot(): ModuleWithProviders<AMod>;
  static forRoot(x: number): ModuleWithProviders<BMod>;
  static forRoot(x?: number): any { return { ngModule: CMod, providers: [] }; }
}
",
        );
        write(
            &dir,
            "types.d.ts",
            r"
import * as i0 from '@angular/core';
export declare class AMod { static ɵmod: i0.ɵɵNgModuleDeclaration<AMod, never, never, never>; }
export declare class BMod { static ɵmod: i0.ɵɵNgModuleDeclaration<BMod, never, never, never>; }
export declare class Typed {
  static forRoot(): i0.ModuleWithProviders<AMod>;
  static forRoot(x: number): i0.ModuleWithProviders<BMod>;
}
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { Holder } from './lib';
import { Typed } from './types';
@NgModule({ imports: [Holder.forRoot(1), Typed.forRoot(1)] })
export class M {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("type:M,imports:[i1.AMod,i2.AMod]}"), "{code}");
    }

    #[test]
    fn imported_returned_calls_and_unreadable_bodies() {
        // ngtsc partial gives the same list as in one file, through `i1`.
        let dir = TempDir::new().unwrap();
        write(&dir, "holder.ts", super::SHAPES);
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { H } from './holder';
@NgModule({ imports: [H.sat(), H.cast(), H.getter(), H.method(), H.paren(), H.wrapTyped(), H.wrapUntyped(), H.wrapDynamic()] })
export class M {}
",
            CompilationMode::Partial,
        );
        assert!(
            code.contains(
                "type:M,imports:[i1.DepModule,i1.DepModule,i1.DepModule,i1.DepModule,i1.EmptyModule,i1.OtherModule,i1.OtherModule,i1.DepModule]}"
            ),
            "{code}"
        );
    }

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

    /// A module, a holder, and lists of them, for imports by name and
    /// through a namespace.
    const DEP_LISTS: &str = r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class DepModule {
  static forRoot(): ModuleWithProviders<DepModule> { return { ngModule: DepModule, providers: [] }; }
}
@NgModule({})
export class OtherModule {}
export class Holder {
  static forRoot(): ModuleWithProviders<OtherModule> { return { ngModule: OtherModule, providers: [] }; }
}
export const mwpConst = DepModule.forRoot();
export const litConst: ModuleWithProviders<OtherModule> = { ngModule: OtherModule, providers: [] };
export const DEP_MODS = [DepModule.forRoot(), OtherModule];
export const NS_LIST = [OtherModule];
";

    #[test]
    fn namespace_members_name_the_module_through_an_import() {
        // ngtsc adds `import * as i1 from "./dep"` next to `ns`. The
        // injector keeps the entries as written.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP_LISTS);
        let source = r"
import { NgModule } from '@angular/core';
import * as ns from './dep';
const localViaNs = ns.Holder.forRoot();
@NgModule({ imports: [ns.DepModule.forRoot(), ns.Holder.forRoot(), ns.mwpConst, ns.litConst, localViaNs, ns.OtherModule] })
export class AppModule {}
";
        let code = compile(&dir, "app.ts", source, CompilationMode::Partial);
        assert!(code.contains("import*asi1from'./dep';"), "{code}");
        assert!(
            code.contains(
                "type:AppModule,imports:[i1.DepModule,i1.OtherModule,i1.DepModule,i1.OtherModule,i1.OtherModule,i1.OtherModule]}"
            ),
            "{code}"
        );
        assert!(
            code.contains(
                "type:AppModule,imports:[ns.DepModule.forRoot(),ns.Holder.forRoot(),ns.mwpConst,ns.litConst,localViaNs,ns.OtherModule]}"
            ),
            "ɵinj should keep the entries as written:\n{code}"
        );
        // Without cross-file analysis, as before.
        let code = partial(source);
        assert!(code.contains("type:AppModule,imports:[localViaNs]}"), "{code}");
    }

    #[test]
    fn imported_lists_are_flattened() {
        // A name this file writes stays as written; a class read in
        // `dep.ts` or through `ns` gets `i1.`.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP_LISTS);
        let source = r"
import { NgModule, Component } from '@angular/core';
import { DepModule, OtherModule, DEP_MODS } from './dep';
import * as ns from './dep';
@Component({ selector: 'sa', template: '', standalone: true })
export class Standalone {}
const LOCAL = [DepModule, OtherModule, Standalone];
const WITH_NS = [ns.DepModule, ns.NS_LIST, DEP_MODS];
const LOCAL_MWP = [DepModule.forRoot()];
@NgModule({ imports: [...DEP_MODS, DepModule] })
export class SpreadModule {}
@NgModule({ imports: [...ns.DEP_MODS] })
export class NsModule {}
@NgModule({ imports: [[DepModule.forRoot()]] })
export class NestedImported {}
@NgModule({ imports: [...LOCAL, WITH_NS, ...LOCAL_MWP, ns.OtherModule] })
export class MixedModule {}
@NgModule({ imports: [ns.DEP_MODS, [ns.DepModule.forRoot()]], exports: [ns.OtherModule] })
export class NsOnly {}
";
        let code = compile(&dir, "app.ts", source, CompilationMode::Partial);
        assert!(
            code.contains("type:SpreadModule,imports:[i1.DepModule,i1.OtherModule,DepModule]}"),
            "{code}"
        );
        assert!(code.contains("type:NsModule,imports:[i1.DepModule,i1.OtherModule]}"), "{code}");
        assert!(code.contains("type:NestedImported,imports:[i1.DepModule]}"), "{code}");
        assert!(
            code.contains(
                "type:MixedModule,imports:[DepModule,OtherModule,Standalone,i1.DepModule,i1.OtherModule,i1.DepModule,i1.OtherModule,i1.DepModule,i1.OtherModule]}"
            ),
            "{code}"
        );
        assert!(
            code.contains(
                "type:NsOnly,imports:[i1.DepModule,i1.OtherModule,i1.DepModule],exports:[i1.OtherModule]}"
            ),
            "{code}"
        );
        // Without cross-file analysis an imported name inside a list is
        // unknown, so those entries keep their old output.
        let code = partial(source);
        assert!(code.contains("type:SpreadModule,imports:[DepModule]}"), "{code}");
        assert!(code.contains("type:MixedModule,imports:[WITH_NS]}"), "{code}");
        assert!(code.contains("type:NsOnly})"), "{code}");
    }

    #[test]
    fn values_oxc_does_not_evaluate_keep_their_old_output() {
        // ngtsc also reads the `let` list and the function call:
        // `[AModule, i1.OtherModule, i1.OtherModule, AModule, AModule]`, and
        // `[AModule]` for the condition.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dep.ts",
            r"
import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class DepModule {}
@NgModule({})
export class OtherModule {}
export const LIT: ModuleWithProviders<OtherModule> = { ngModule: OtherModule, providers: [] };
export const LIT_LIST = [{ ngModule: OtherModule, providers: [] }];
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { LIT, LIT_LIST, DepModule } from './dep';
@NgModule({})
export class AModule {}
let LET_LIST = [AModule];
@NgModule({ imports: [{ ngModule: AModule, providers: [] }, LIT, ...LIT_LIST, ...LET_LIST, [forwardRefLess()]] })
export class AppModule {}
function forwardRefLess() { return AModule; }
@NgModule({ imports: [...(cond() ? [AModule] : [DepModule])] })
export class CondModule {}
function cond() { return true; }
",
            CompilationMode::Partial,
        );
        assert!(
            code.contains("type:AppModule,imports:[AModule,i1.OtherModule,i1.OtherModule]}"),
            "{code}"
        );
        assert!(code.contains("type:CondModule})"), "{code}");
    }

    #[test]
    fn namespaces_are_numbered_in_list_order() {
        // ngtsc: `import * as i1 from "./dep2"; import * as i2 from "./dep"`.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP_LISTS);
        write(
            &dir,
            "dep2.ts",
            r"
import { NgModule } from '@angular/core';
@NgModule({})
export class TwoModule {}
export const L2 = [TwoModule];
",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import * as ns1 from './dep';
import * as ns2 from './dep2';
@NgModule({ imports: [...ns2.L2, ...ns1.DEP_MODS] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("import*asi1from'./dep2';import*asi2from'./dep';"), "{code}");
        assert!(
            code.contains("type:AppModule,imports:[i1.TwoModule,i2.DepModule,i2.OtherModule]}"),
            "{code}"
        );
    }

    #[test]
    fn a_class_is_named_by_its_export_from_its_own_file() {
        // ngtsc names `Inner` by its export `Public` of `./dep`, even
        // through a barrel. A class that isn't exported can't be named
        // (ngtsc error -993004): that list keeps its old output.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dep.ts",
            r"
import { NgModule } from '@angular/core';
@NgModule({})
class Inner {}
export { Inner as Public };
export const DEP_MODS = [Inner];
@NgModule({})
class Hidden {}
export const HIDDEN_MODS = [Hidden];
",
        );
        write(&dir, "barrel.ts", "export * from './dep';\n");
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { DEP_MODS, HIDDEN_MODS } from './dep';
import * as b from './barrel';
@NgModule({ imports: [...DEP_MODS] })
export class AppModule {}
@NgModule({ imports: [b.Public, ...b.DEP_MODS] })
export class BarrelModule {}
@NgModule({ imports: [...HIDDEN_MODS] })
export class HiddenModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("import*asi1from'./dep';"), "{code}");
        assert!(!code.contains("import*asi2"), "{code}");
        assert!(code.contains("type:AppModule,imports:[i1.Public]}"), "{code}");
        assert!(code.contains("type:BarrelModule,imports:[i1.Public,i1.Public]}"), "{code}");
        assert!(code.contains("type:HiddenModule})"), "{code}");
    }

    #[test]
    fn an_aliased_import_in_a_list_stays_as_written() {
        // ngtsc partial: `imports: [D, i1.DepModule, D]`.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP_LISTS);
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { DepModule as D } from './dep';
const LOCAL = [D, D.forRoot()];
@NgModule({ imports: [...LOCAL, [D]] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("type:AppModule,imports:[D,i1.DepModule,D]}"), "{code}");
    }

    #[test]
    fn an_import_specifier_is_kept_as_written() {
        // ngtsc writes `"./dep"`; oxc keeps the file's own `./dep.js`.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP_LISTS);
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import * as ns from './dep.js';
@NgModule({ imports: [...ns.NS_LIST] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("import*asi1from'./dep.js';"), "{code}");
        assert!(code.contains("type:AppModule,imports:[i1.OtherModule]}"), "{code}");
    }

    const APP_FORWARD_REFS: &str = r"
import { NgModule, forwardRef } from '@angular/core';
import { DepModule, DEP_MODS, PLAIN_MODS } from './dep';
@NgModule({ imports: [forwardRef(() => DepModule)] })
export class AppModule {}
@NgModule({ imports: [...DEP_MODS] })
export class ListModule {}
@NgModule({ imports: [...PLAIN_MODS] })
export class PlainListModule {}
";

    const DEP_FORWARD_REFS: &str = r"
import { NgModule, forwardRef } from '@angular/core';
@NgModule({})
export class DepModule {}
@NgModule({})
export class OtherModule {}
export const DEP_MODS = [forwardRef(() => OtherModule)];
export const PLAIN_MODS = [OtherModule];
";

    #[test]
    fn a_forward_ref_to_another_file_is_not_wrapped() {
        // ngtsc partial: `[DepModule]`, `[i1.OtherModule]`, `[i1.OtherModule]`.
        let dir = TempDir::new().unwrap();
        write(&dir, "dep.ts", DEP_FORWARD_REFS);
        let code = compile(&dir, "app.ts", APP_FORWARD_REFS, CompilationMode::Partial);
        assert!(code.contains("type:AppModule,imports:[DepModule]}"), "{code}");
        assert!(code.contains("type:ListModule,imports:[i1.OtherModule]}"), "{code}");
        assert!(code.contains("type:PlainListModule,imports:[i1.OtherModule]}"), "{code}");
        let code = compile(&dir, "app.ts", APP_FORWARD_REFS, CompilationMode::Full);
        assert!(code.contains("ɵɵsetNgModuleScope(AppModule,{imports:[DepModule]})"), "{code}");
    }

    #[test]
    fn a_forward_ref_to_a_file_that_loads_this_one_stays_wrapped() {
        // `dep.ts` imports this file back, so it may not have run when this
        // file defines its NgModules. ngtsc reads the classes in place
        // (`[DepModule]`, `[i1.OtherModule]`); oxc keeps them lazy where a
        // `forwardRef` asked for it. A list without one is read in place.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "dep.ts",
            &format!(
                "{DEP_FORWARD_REFS}import {{ AppModule }} from './app';\nexport const USE = AppModule;\n"
            ),
        );
        let code = compile(&dir, "app.ts", APP_FORWARD_REFS, CompilationMode::Partial);
        assert!(code.contains("type:AppModule,imports:()=>[DepModule]}"), "{code}");
        assert!(code.contains("type:ListModule,imports:()=>[i1.OtherModule]}"), "{code}");
        assert!(code.contains("type:PlainListModule,imports:[i1.OtherModule]}"), "{code}");
        let code = compile(&dir, "app.ts", APP_FORWARD_REFS, CompilationMode::Full);
        assert!(code.contains("ɵɵsetNgModuleScope(AppModule,{imports:()=>[DepModule]})"), "{code}");
    }

    #[test]
    fn a_return_type_can_import_a_new_file() {
        // The holder's return type names a module this file never imports.
        // ngtsc imports it: `import * as i1 from "./dep"`, `[i1.DepModule]`.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "holder.ts",
            r"
import { ModuleWithProviders } from '@angular/core';
import type { DepModule } from './dep';
export class Holder {
  static forRoot(): ModuleWithProviders<DepModule> { const p: any[] = []; return { ngModule: null as any, providers: p }; }
}
",
        );
        write(
            &dir,
            "dep.ts",
            "import { NgModule } from '@angular/core';\n@NgModule({})\nexport class DepModule {}\n",
        );
        let code = compile(
            &dir,
            "app.ts",
            r"
import { NgModule } from '@angular/core';
import { Holder } from './holder';
@NgModule({ imports: [Holder.forRoot()] })
export class AppModule {}
",
            CompilationMode::Partial,
        );
        assert!(code.contains("import*asi1from'./dep';"), "{code}");
        assert!(code.contains("type:AppModule,imports:[i1.DepModule]}"), "{code}");
    }
}

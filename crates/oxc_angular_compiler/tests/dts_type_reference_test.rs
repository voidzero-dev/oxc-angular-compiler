//! Class references in `.d.ts` members (`ɵmod` lists, `hostDirectives`)
//! follow ngtsc's type reference rule (#692): `toR3Reference` emits them
//! with `ImportFlags.ForceNewImport`, so a class this file declares keeps
//! its name and any other is its export through a namespace import of the
//! `.d.ts` (`typeof i1.Dep`, `typeof i1.default`). The `.d.ts` numbers its
//! own `iN` by first use, apart from the JS.
//!
//! Expected text is what `@angular/compiler-cli` 22.2.3 writes for the same
//! sources, except that a barrel is kept as written.

use std::collections::HashMap;

use oxc_allocator::Allocator;
use oxc_angular_compiler::{
    CompilationMode, TransformOptions, TransformResult, transform_angular_file,
};

const MODES: [CompilationMode; 2] = [CompilationMode::Full, CompilationMode::Partial];

fn transform(source: &str, mode: CompilationMode) -> TransformResult {
    let allocator = Allocator::default();
    let options = TransformOptions { compilation_mode: mode, ..Default::default() };
    let result = transform_angular_file(&allocator, "src/test.ts", source, Some(&options), None);
    assert!(!result.has_errors(), "{:?}", result.diagnostics);
    result
}

/// The members and namespace imports of `class_name` in the `.d.ts`.
fn dts<'r>(
    result: &'r TransformResult,
    class_name: &str,
) -> (&'r str, &'r HashMap<String, String>) {
    let decl = result
        .dts_declarations
        .iter()
        .find(|decl| decl.class_name == class_name)
        .unwrap_or_else(|| panic!("no .d.ts for {class_name}"));
    (&decl.members, &decl.namespace_imports)
}

fn map(entries: &[(&str, &str)]) -> HashMap<String, String> {
    entries.iter().map(|(alias, module)| ((*alias).to_string(), (*module).to_string())).collect()
}

/// The modules the JS imports as a namespace.
fn js_namespace_imports(code: &str) -> Vec<&str> {
    code.lines()
        .filter(|line| line.starts_with("import * as "))
        .filter_map(|line| line.split(" from ").nth(1))
        .collect()
}

const NG_MODULE: &str = r"
import { NgModule, Component, forwardRef } from '@angular/core';
import { CommonModule } from '@angular/common';
import { HelloComponent } from './hello.component';
import { OtherModule as OM } from './other.module';
import DefDirective from './def.directive';
import { MwpModule } from './mwp.module';

@Component({ selector: 'internal', template: '', standalone: false })
class InternalComponent {}

@NgModule({
  declarations: [InternalComponent, HelloComponent],
  imports: [CommonModule, OM, DefDirective, MwpModule.forRoot(), forwardRef(() => Later), Unknown],
  exports: [InternalComponent, HelloComponent, OM],
})
export class LibModule {}

@Component({ selector: 'later', template: '' })
export class Later {}
";

#[test]
fn ng_module_lists_use_type_references() {
    for mode in MODES {
        let result = transform(NG_MODULE, mode);
        let (members, imports) = dts(&result, "LibModule");
        assert!(
            members.contains(
                "static ɵmod: i0.ɵɵNgModuleDeclaration<LibModule, \
                 [typeof InternalComponent, typeof i1.HelloComponent], \
                 [typeof i2.CommonModule, typeof i3.OtherModule, typeof i4.default, \
                 typeof i5.MwpModule, typeof Later, typeof Unknown], \
                 [typeof InternalComponent, typeof i1.HelloComponent, typeof i3.OtherModule]>;"
            ),
            "{mode:?}:\n{members}"
        );
        assert_eq!(
            imports,
            &map(&[
                ("i0", "@angular/core"),
                ("i1", "./hello.component"),
                ("i2", "@angular/common"),
                ("i3", "./other.module"),
                ("i4", "./def.directive"),
                ("i5", "./mwp.module"),
            ]),
            "{mode:?}"
        );
    }
}

#[test]
fn ng_module_type_references_add_no_js_import() {
    for mode in MODES {
        let result = transform(NG_MODULE, mode);
        assert_eq!(js_namespace_imports(&result.code), ["'@angular/core';"], "{mode:?}");
    }
}

#[test]
fn component_host_directives_use_type_references() {
    let source = r"
import { Component, Directive } from '@angular/core';
import { HighlightDirective as HD } from './highlight.directive';
import DefDirective from './def.directive';

@Directive({ selector: '[local]' })
export class LocalDir {}

@Component({
  selector: 'host-cmp',
  template: '',
  hostDirectives: [{ directive: HD, inputs: ['color'] }, LocalDir, DefDirective],
})
export class HostCmp {}
";
    for mode in MODES {
        let result = transform(source, mode);
        let (members, imports) = dts(&result, "HostCmp");
        assert!(
            members.contains(
                "[{ directive: typeof i1.HighlightDirective; inputs: { \"color\": \"color\" }; outputs: {}; }, \
                 { directive: typeof LocalDir; inputs: {}; outputs: {}; }, \
                 { directive: typeof i2.default; inputs: {}; outputs: {}; }]"
            ),
            "{mode:?}:\n{members}"
        );
        assert_eq!(
            imports,
            &map(&[
                ("i0", "@angular/core"),
                ("i1", "./highlight.directive"),
                ("i2", "./def.directive"),
            ]),
            "{mode:?}"
        );
    }
}

/// The JS names `./svc` first (`i1`), the `.d.ts` only the host directive.
#[test]
fn directive_host_directives_use_their_own_namespaces() {
    let source = r"
import { Directive, Optional } from '@angular/core';
import { Svc } from './svc';
import { HighlightDirective as HD } from './highlight.directive';

@Directive({ selector: '[c]', hostDirectives: [HD] })
export class CDir {
  constructor(@Optional() s: Svc) {}
}
";
    for mode in MODES {
        let result = transform(source, mode);
        let (members, imports) = dts(&result, "CDir");
        assert!(
            members.contains(
                "[{ directive: typeof i1.HighlightDirective; inputs: {}; outputs: {}; }]>;"
            ),
            "{mode:?}:\n{members}"
        );
        assert_eq!(
            imports,
            &map(&[("i0", "@angular/core"), ("i1", "./highlight.directive")]),
            "{mode:?}"
        );
        assert!(result.code.contains("i2.HighlightDirective"), "{mode:?}:\n{}", result.code);
        assert_eq!(
            js_namespace_imports(&result.code),
            ["'@angular/core';", "'./svc';", "'./highlight.directive';"],
            "{mode:?}"
        );
    }
}

/// The `.d.ts` names an export written as a string by that string: ngtsc
/// writes `typeof i1.Plain` for `"Plain"`, and `typeof i1.s-dir`, which
/// does not parse, for `"s-dir"`; here it is `typeof i1["s-dir"]`, which
/// TS reads as `(typeof i1)["s-dir"]`.
#[test]
fn string_export_names_use_type_references() {
    let source = r#"
import { Component, NgModule } from '@angular/core';
import { "s-dir" as SDir, "Plain" as P } from './dep/str';

@Component({ selector: 'app', template: '', hostDirectives: [SDir, P] })
export class App {}

@NgModule({ imports: [SDir, P], exports: [SDir] })
export class M {}
"#;
    for mode in MODES {
        let result = transform(source, mode);
        let (members, imports) = dts(&result, "App");
        assert!(
            members.contains(
                "[{ directive: typeof i1[\"s-dir\"]; inputs: {}; outputs: {}; }, \
                 { directive: typeof i1.Plain; inputs: {}; outputs: {}; }]>;"
            ),
            "{mode:?}:\n{members}"
        );
        assert_eq!(imports, &map(&[("i0", "@angular/core"), ("i1", "./dep/str")]), "{mode:?}");
        let (members, _) = dts(&result, "M");
        assert!(
            members.contains(
                "ɵɵNgModuleDeclaration<M, never, [typeof i1[\"s-dir\"], typeof i1.Plain], \
                 [typeof i1[\"s-dir\"]]>;"
            ),
            "{mode:?}:\n{members}"
        );
    }
}
#[test]
fn nested_class_takes_no_dts_alias() {
    let source = r"
import { NgModule, Directive } from '@angular/core';
import { CommonModule } from '@angular/common';
import { HighlightDirective } from './highlight.directive';

export function f() {
  @NgModule({ imports: [CommonModule] })
  class Nested {}
  return Nested;
}

@Directive({ selector: '[x]', hostDirectives: [HighlightDirective] })
export class X {}
";
    for mode in MODES {
        let result = transform(source, mode);
        assert!(result.dts_declarations.iter().all(|decl| decl.class_name != "Nested"));
        let (members, imports) = dts(&result, "X");
        assert!(members.contains("typeof i1.HighlightDirective"), "{mode:?}:\n{members}");
        assert_eq!(
            imports,
            &map(&[("i0", "@angular/core"), ("i1", "./highlight.directive")]),
            "{mode:?}"
        );
    }
}

/// A `namespace` member is in the `.d.ts`: its lists are read as written.
#[test]
fn namespace_member_ng_module_uses_type_references() {
    let source = r"
import { NgModule } from '@angular/core';
import { CommonModule } from '@angular/common';

export namespace NS {
  @NgModule({ imports: [CommonModule] })
  export class M {}
}
";
    for mode in MODES {
        let result = transform(source, mode);
        let (members, imports) = dts(&result, "NS.M");
        assert!(
            members.contains("ɵɵNgModuleDeclaration<M, never, [typeof i1.CommonModule], never>;"),
            "{mode:?}:\n{members}"
        );
        assert_eq!(
            imports,
            &map(&[("i0", "@angular/core"), ("i1", "@angular/common")]),
            "{mode:?}"
        );
    }
}

/// With cross-file analysis, the lists are flattened like ngtsc's
/// `resolveTypeList`: spreads, `const` lists, `ns.X` and a
/// `ModuleWithProviders` type.
#[cfg(feature = "cross_file_elision")]
mod cross_file {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path
    }

    fn transform_in(dir: &TempDir, source: &str, mode: CompilationMode) -> TransformResult {
        let path = write(dir.path(), "src/test.ts", source);
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
        result
    }

    fn project() -> TempDir {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        write(
            root,
            "src/common.ts",
            "import { NgModule, Directive, Pipe } from '@angular/core';
@Directive({ selector: '[ngIf]' })
export class NgIf {}
@Pipe({ name: 'date' })
export class DatePipe {}
@NgModule({ imports: [NgIf, DatePipe], exports: [NgIf, DatePipe] })
export class CommonModule {}
",
        );
        write(
            root,
            "src/highlight.directive.ts",
            "import { Directive } from '@angular/core';
@Directive({ selector: '[hl]' })
export class HighlightDirective {}
",
        );
        write(
            root,
            "src/def.directive.ts",
            "import { Directive } from '@angular/core';
@Directive({ selector: '[def]' })
export default class DefDirective {}
",
        );
        write(
            root,
            "src/mwp.module.ts",
            "import { NgModule, ModuleWithProviders } from '@angular/core';
@NgModule({})
export class MwpModule {
  static forRoot(): ModuleWithProviders<MwpModule> { return { ngModule: MwpModule, providers: [] }; }
}
",
        );
        write(
            root,
            "src/dirs.ts",
            "import { HighlightDirective } from './highlight.directive';
export const HOST_DIRS = [HighlightDirective];
",
        );
        dir
    }

    #[test]
    fn flattened_lists_match_ngtsc() {
        let dir = project();
        let source = r"
import { NgModule } from '@angular/core';
import { CommonModule } from './common';
import * as cm from './common';
import { HighlightDirective as HD } from './highlight.directive';
import { MwpModule } from './mwp.module';
import DefDirective from './def.directive';
const LOCAL = [HD, cm.NgIf];
@NgModule({ imports: [CommonModule, ...LOCAL, MwpModule.forRoot(), DefDirective], exports: [LOCAL, cm.DatePipe] })
export class EModule {}
";
        for mode in MODES {
            let result = transform_in(&dir, source, mode);
            let (members, imports) = dts(&result, "EModule");
            assert!(
                members.contains(
                    "ɵɵNgModuleDeclaration<EModule, never, \
                     [typeof i1.CommonModule, typeof i2.HighlightDirective, typeof i1.NgIf, \
                     typeof i3.MwpModule, typeof i4.default], \
                     [typeof i2.HighlightDirective, typeof i1.NgIf, typeof i1.DatePipe]>;"
                ),
                "{mode:?}:\n{members}"
            );
            assert_eq!(
                imports,
                &map(&[
                    ("i0", "@angular/core"),
                    ("i1", "./common"),
                    ("i2", "./highlight.directive"),
                    ("i3", "./mwp.module"),
                    ("i4", "./def.directive"),
                ]),
                "{mode:?}"
            );
        }
    }

    /// An imported list names its classes through the file they come from.
    #[test]
    fn imported_list_names_its_classes() {
        let dir = project();
        let source = r"
import { NgModule } from '@angular/core';
import { HOST_DIRS } from './dirs';
@NgModule({ imports: HOST_DIRS, exports: [HOST_DIRS] })
export class XM {}
";
        for mode in MODES {
            let result = transform_in(&dir, source, mode);
            let (members, imports) = dts(&result, "XM");
            assert!(
                members.contains(
                    "ɵɵNgModuleDeclaration<XM, never, [typeof i1.HighlightDirective], \
                     [typeof i1.HighlightDirective]>;"
                ),
                "{mode:?}:\n{members}"
            );
            assert_eq!(
                imports,
                &map(&[("i0", "@angular/core"), ("i1", "./highlight.directive")]),
                "{mode:?}"
            );
        }
    }
}

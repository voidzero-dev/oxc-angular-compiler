//! A constructor parameter typed by a `const enum` gets `type: undefined` in
//! the class metadata (full `ɵsetClassMetadata`, partial
//! `ɵɵngDeclareClassMetadata`), like ngtsc's `typeToValue` outside local
//! compilation (issue #623). Each expected output was checked against
//! `@angular/compiler-cli` 22.2.3 in full and partial mode.
//!
//! - same file: `const enum`, `declare const enum`, `ns.E`, and a `const enum`
//!   merged with namespaces that hold only types
//! - imported: only with `resolve_imported_values`, when the other file can
//!   be read and shows a `const enum`: named, default and namespace imports
//!   (`k.KeyCodes`, `k.NS.Inner`), re-exports, `.d.ts` files and package
//!   typings
//! - anything oxc can't be sure of keeps today's output, and factories don't
//!   change

// The expected output has object literals like `{type:Foo}`.
#![expect(clippy::literal_string_with_formatting_args)]

use oxc_allocator::Allocator;
use oxc_angular_compiler::{CompilationMode, TransformOptions, transform_angular_file};

const MODES: [CompilationMode; 2] = [CompilationMode::Full, CompilationMode::Partial];

/// The compiled `source`, without whitespace.
fn compile_at(path: &str, source: &str, options: &TransformOptions) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, path, source, Some(options), None);
    assert!(!result.has_errors(), "Should not have errors: {:?}", result.diagnostics);
    result.code.chars().filter(|c| !c.is_whitespace()).collect()
}

fn compile(source: &str, compilation_mode: CompilationMode) -> String {
    compile_at("test.ts", source, &TransformOptions { compilation_mode, ..Default::default() })
}

/// The `ctorParameters` arrow body of the first class metadata in `code`.
fn ctor_params(code: &str) -> &str {
    let start = code.find("()=>[{type:").expect("ctor parameters") + "()=>".len();
    let mut depth = 0;
    for (i, c) in code[start..].char_indices() {
        match c {
            '[' | '{' | '(' => depth += 1,
            ']' | '}' | ')' => {
                depth -= 1;
                if depth == 0 {
                    return &code[start..=start + i];
                }
            }
            _ => {}
        }
    }
    &code[start..]
}

fn assert_contains(code: &str, expected: &str) {
    assert!(code.contains(expected), "Expected `{expected}` in:\n{code}");
}

fn assert_not_contains(code: &str, unexpected: &str) {
    assert!(!code.contains(unexpected), "Did not expect `{unexpected}` in:\n{code}");
}

#[test]
fn same_file_const_enum_param_has_no_type() {
    // ngtsc: `type: undefined` / `type: Plain` / `type: undefined`.
    let source = "import { Injectable, Inject } from '@angular/core';
export const enum KeyCodes { A = 1, B = 2 }
export enum Plain { X = 1 }
@Injectable()
export class Svc {
  constructor(@Inject('KC') kc: KeyCodes, @Inject('P') p: Plain, @Inject('N') n: KeyCodes | null) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]},\
             {type:Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"N\"]}]}]",
            "{mode:?}"
        );
    }
}

#[test]
fn declare_and_local_const_enum_params_have_no_type() {
    // A `declare const enum` is erased: `type: Local` threw a ReferenceError
    // when JIT or TestBed read the metadata. ngtsc: `undefined` for both.
    let source = "import { Injectable, Inject } from '@angular/core';
declare const enum Local { A = 1 }
const enum NotExported { A = 1 }
@Injectable()
export class Svc {
  constructor(@Inject('KC') kc: Local, @Inject('NE') ne: NotExported) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"NE\"]}]}]",
            "{mode:?}"
        );
    }
}

#[test]
fn namespace_const_enum_param_has_no_type() {
    // ngtsc: `undefined`, `NS.P`, `undefined`, `undefined`.
    let source = "import { Injectable, Inject } from '@angular/core';
export namespace NS { export const enum E { A = 1 } export enum P { A = 1 } export namespace Deep { export const enum D { A = 1 } } }
declare namespace Amb { const enum E { A = 1 } }
@Injectable()
export class Svc {
  constructor(@Inject('E') e: NS.E, @Inject('P') p: NS.P, @Inject('D') d: NS.Deep.D, @Inject('A') a: Amb.E) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:undefined,decorators:[{type:Inject,args:[\"E\"]}]},\
             {type:NS.P,decorators:[{type:Inject,args:[\"P\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"D\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"A\"]}]}]",
            "{mode:?}"
        );
    }
}

#[test]
fn directive_and_component_const_enum_params_have_no_type() {
    let source = "import { Component, Directive, Inject, Optional } from '@angular/core';
const enum KeyCodes { A = 1 }
@Directive({ selector: '[d]' })
export class Dir {
  constructor(@Inject('KC') @Optional() kc: KeyCodes) {}
}
@Component({ selector: 'c', template: '' })
export class Cmp {
  constructor(@Inject('KC') kc: KeyCodes) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_contains(
            &code,
            "()=>[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]},{type:Optional}]}]",
        );
        assert_contains(&code, "()=>[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]}]");
        assert_not_contains(&code, "type:KeyCodes");
    }
}

#[test]
fn const_enum_merged_with_a_type_only_namespace_has_no_type() {
    // The merged symbol keeps its `ConstEnum` flag. ngtsc: `undefined` six
    // times, then `R1` for a regular enum.
    let source = "import { Injectable, Inject } from '@angular/core';
const enum T1 { A = 1 }
namespace T1 { export type X = 1; }
namespace T2 { export interface I {} }
const enum T2 { A = 1 }
declare const enum D1 { A = 1 }
declare namespace D1 { type X = 1; }
export const enum E1 { A = 1 }
export namespace E1 { export namespace Deep { export type Z = 1; } }
const enum N1 { A = 1 }
namespace N1 { }
export namespace Outer { export const enum M { A = 1 } export namespace M { export type Q = 1; } }
enum R1 { A = 1 }
namespace R1 { export const x = 1; }
@Injectable()
export class Svc {
  constructor(@Inject('T1') t1: T1, @Inject('T2') t2: T2, @Inject('D1') d1: D1, @Inject('E1') e1: E1, @Inject('N1') n1: N1, @Inject('OM') om: Outer.M, @Inject('R1') r1: R1) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:undefined,decorators:[{type:Inject,args:[\"T1\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"T2\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"D1\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"E1\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"N1\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"OM\"]}]},\
             {type:R1,decorators:[{type:Inject,args:[\"R1\"]}]}]",
            "{mode:?}"
        );
    }
}

#[test]
fn const_enum_merged_over_several_namespace_blocks_has_no_type() {
    // Two type-only blocks; a block with only a local `import X = A.B`; a
    // block of `Outer` that keeps an instantiated `namespace M3` to itself
    // (it doesn't merge with the exported `M3`); a dotted `namespace D.B`.
    // ngtsc: `undefined` for all four.
    let source = "import { Injectable, Inject } from '@angular/core';
const enum M { A = 1 }
namespace M { export type T = 1; }
namespace M { export interface I {} }
namespace Foo { export namespace Bar { export type Z = 1; } }
const enum M2 { A = 1 }
namespace M2 { import X = Foo.Bar; export type Y = X.Z; }
export namespace Outer { export const enum M3 { A = 1 } }
export namespace Outer { namespace M3 { export const x = 1; } export const y = M3.x; }
const enum D { A = 1 }
namespace D.B { export type X = 1; }
@Injectable()
export class Svc {
  constructor(@Inject('M') m: M, @Inject('M2') m2: M2, @Inject('M3') m3: Outer.M3, @Inject('D') d: D) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:undefined,decorators:[{type:Inject,args:[\"M\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"M2\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"M3\"]}]},\
             {type:undefined,decorators:[{type:Inject,args:[\"D\"]}]}]",
            "{mode:?}"
        );
    }
}

#[test]
fn const_enum_merged_with_an_instantiated_namespace_keeps_its_type() {
    // TS2567: ngtsc writes no output. The reference stays as before.
    let source = "import { Injectable, Inject } from '@angular/core';
const enum V1 { A = 1 }
namespace V1 { export const x = 1; }
declare const enum V2 { A = 1 }
declare namespace V2 { const x: number; }
const enum V3 { A = 1 }
namespace V3 { export const enum I { B = 2 } }
@Injectable()
export class Svc {
  constructor(@Inject('V1') v1: V1, @Inject('V2') v2: V2, @Inject('V3') v3: V3) {}
}";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:V1,decorators:[{type:Inject,args:[\"V1\"]}]},\
             {type:V2,decorators:[{type:Inject,args:[\"V2\"]}]},\
             {type:V3,decorators:[{type:Inject,args:[\"V3\"]}]}]",
            "{mode:?}"
        );
    }
}

#[test]
fn const_enum_factory_dep_is_unchanged() {
    // Only the metadata `type:` follows ngtsc; the factory keeps its token
    // (ngtsc: `ɵɵinvalidFactory()` / `deps: "invalid"`).
    let source = "import { Injectable } from '@angular/core';
const enum Local { A = 1 }
@Injectable()
export class Svc { constructor(l: Local) {} }";
    let full = compile(source, CompilationMode::Full);
    assert_contains(&full, "i0.ɵɵinject(Local)");
    assert_eq!(ctor_params(&full), "[{type:undefined}]");
    let partial = compile(source, CompilationMode::Partial);
    assert_contains(&partial, "deps:[{token:Local}]");
    assert_eq!(ctor_params(&partial), "[{type:undefined}]");
}

#[cfg(feature = "cross_file_elision")]
mod cross_file {
    use super::*;
    use tempfile::TempDir;

    fn write(dir: &TempDir, name: &str, content: &str) -> String {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        path.to_string_lossy().into_owned()
    }

    fn options(mode: CompilationMode, resolve_imported_values: bool) -> TransformOptions {
        TransformOptions { compilation_mode: mode, resolve_imported_values, ..Default::default() }
    }

    const KEYS: &str = "export const enum KeyCodes { A = 1, B = 2 }
export enum Plain { X = 1 }
const enum Local { A = 1 }
export { Local };
export declare const enum Decl { A = 1 }
";

    const IMPORTER: &str = "import { Injectable, Inject } from '@angular/core';
import { KeyCodes, Plain, KeyCodes as KC, Local, Decl } from './keys';
@Injectable()
export class Svc {
  constructor(@Inject('KC') kc: KeyCodes, @Inject('P') p: Plain, @Inject('A') a: KC, @Inject('L') l: Local, @Inject('D') d: Decl) {}
}";

    #[test]
    fn imported_const_enum_param_has_no_type() {
        // ngtsc: `undefined`, `i1.Plain`, then `undefined` three times.
        let dir = TempDir::new().unwrap();
        write(&dir, "keys.ts", KEYS);
        let path = write(&dir, "a.ts", IMPORTER);
        for mode in MODES {
            let code = compile_at(&path, IMPORTER, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]},\
                 {type:i1.Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"A\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"L\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"D\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn imported_const_enum_file_is_a_dependency() {
        let dir = TempDir::new().unwrap();
        let keys = write(&dir, "keys.ts", KEYS);
        let path = write(&dir, "a.ts", IMPORTER);
        let allocator = Allocator::default();
        let result = transform_angular_file(
            &allocator,
            &path,
            IMPORTER,
            Some(&options(CompilationMode::Full, true)),
            None,
        );
        let keys = std::fs::canonicalize(keys).unwrap();
        assert!(
            result
                .dependencies
                .iter()
                .any(|dep| std::fs::canonicalize(dep).is_ok_and(|dep| dep == keys)),
            "{:?}",
            result.dependencies
        );
    }

    #[test]
    fn imported_const_enum_keeps_its_type_without_the_option() {
        // Default output doesn't change, nor does `cross_file_elision` alone.
        let dir = TempDir::new().unwrap();
        write(&dir, "keys.ts", KEYS);
        let path = write(&dir, "a.ts", IMPORTER);
        for mode in MODES {
            for options in [
                options(mode, false),
                TransformOptions {
                    compilation_mode: mode,
                    cross_file_elision: true,
                    ..Default::default()
                },
            ] {
                let code = compile_at(&path, IMPORTER, &options);
                assert_contains(
                    ctor_params(&code),
                    "[{type:i1.KeyCodes,decorators:[{type:Inject,args:[\"KC\"]}]},",
                );
            }
        }
    }

    #[test]
    fn only_const_enum_import_adds_no_namespace_import() {
        // ngtsc emits no `import * as i1 from "./keys"`.
        let dir = TempDir::new().unwrap();
        write(&dir, "keys.ts", "export const enum KeyCodes { A = 1 }\n");
        let source = "import { Injectable, Inject } from '@angular/core';
import { KeyCodes } from './keys';
@Injectable()
export class Svc {
  constructor(@Inject('KC') kc: KeyCodes) {}
}";
        let path = write(&dir, "a.ts", source);
        for mode in MODES {
            let code = compile_at(&path, source, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]}]",
                "{mode:?}"
            );
            assert_not_contains(&code, "from\"./keys\"");
            assert_not_contains(&code, "from'./keys'");
        }
    }

    #[test]
    fn re_exported_const_enum_param_has_no_type() {
        // ngtsc: `undefined`, `i1.Plain`, `undefined`, `undefined`.
        let dir = TempDir::new().unwrap();
        write(
            &dir,
            "keys.ts",
            "export const enum KeyCodes { A = 1 }\nexport enum Plain { X = 1 }\n",
        );
        write(&dir, "star.ts", "export const enum Star { A = 1 }\n");
        write(
            &dir,
            "barrel.ts",
            "export { KeyCodes, Plain } from './keys';
export { KeyCodes as Renamed } from './keys';
export * from './star';
",
        );
        let source = "import { Injectable, Inject } from '@angular/core';
import { KeyCodes, Plain, Star, Renamed } from './barrel';
@Injectable()
export class Svc {
  constructor(@Inject('KC') kc: KeyCodes, @Inject('P') p: Plain, @Inject('S') s: Star, @Inject('R') r: Renamed) {}
}";
        let path = write(&dir, "a.ts", source);
        for mode in MODES {
            let code = compile_at(&path, source, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]},\
                 {type:i1.Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"S\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"R\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn missing_or_nested_imported_const_enum_keeps_its_type() {
        // A module that can't be read, and a class nested in a function
        // (it may see another declaration of the name): `i{n}.X` stays.
        let dir = TempDir::new().unwrap();
        write(&dir, "keys.ts", "export const enum KeyCodes { A = 1 }\n");
        let source = "import { Injectable, Inject } from '@angular/core';
import * as k from './keys';
import { Missing } from './missing';
@Injectable()
export class Svc {
  constructor(@Inject('M') m: Missing) {}
}
export function make() {
  @Injectable()
  class Nested { constructor(@Inject('K') a: k.KeyCodes) {} }
  return Nested;
}";
        let path = write(&dir, "a.ts", source);
        for mode in MODES {
            let code = compile_at(&path, source, &options(mode, true));
            assert_contains(&code, "()=>[{type:i1.Missing,");
            assert_contains(&code, "()=>[{type:i2.KeyCodes,");
            assert_not_contains(&code, "type:undefined");
        }
    }

    /// Write `files` to a new directory and compile its `a.ts`.
    fn compile_files(files: &[(&str, &str)], options: &TransformOptions) -> String {
        let dir = TempDir::new().unwrap();
        for (name, content) in files {
            write(&dir, name, content);
        }
        let (_, source) = files.iter().find(|(name, _)| *name == "a.ts").unwrap();
        compile_at(&dir.path().join("a.ts").to_string_lossy(), source, options)
    }

    const NS_KEYS: &str = "export const enum KeyCodes { A = 1, B = 2 }
export enum Plain { X = 1 }
export declare const enum Decl { A = 1 }
";

    const NS_IMPORTER: [(&str, &str); 3] = [
        ("keys.ts", NS_KEYS),
        ("barrel.ts", "export * from './keys';\nexport { KeyCodes as Renamed } from './keys';\n"),
        (
            "a.ts",
            "import { Injectable, Inject } from '@angular/core';
import * as k from './keys';
import * as b from './barrel';
@Injectable()
export class Svc {
  constructor(@Inject('K') a: k.KeyCodes, @Inject('P') p: k.Plain, @Inject('D') d: k.Decl, @Inject('BK') bk: b.KeyCodes, @Inject('BR') br: b.Renamed, @Inject('BP') bp: b.Plain, @Inject('NN') nn: k.KeyCodes | null, @Inject('M') m: k.KeyCodes.A) {}
}",
        ),
    ];

    #[test]
    fn namespace_import_const_enum_param_has_no_type() {
        // ngtsc: an enum member (`k.KeyCodes.A`) keeps its reference, and
        // the imports stay `i1` ./keys and `i2` ./barrel.
        for mode in MODES {
            let code = compile_files(&NS_IMPORTER, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:i1.Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"D\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"BK\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"BR\"]}]},\
                 {type:i2.Plain,decorators:[{type:Inject,args:[\"BP\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"NN\"]}]},\
                 {type:i1.KeyCodes.A,decorators:[{type:Inject,args:[\"M\"]}]}]",
                "{mode:?}"
            );
            assert_contains(&code, "import*asi1from'./keys'");
            assert_contains(&code, "import*asi2from'./barrel'");
        }
    }

    #[test]
    fn namespace_import_const_enum_keeps_its_type_without_the_option() {
        for mode in MODES {
            for options in [
                options(mode, false),
                TransformOptions {
                    compilation_mode: mode,
                    cross_file_elision: true,
                    ..Default::default()
                },
            ] {
                let code = compile_files(&NS_IMPORTER, &options);
                let params = ctor_params(&code);
                assert_contains(
                    params,
                    "[{type:i1.KeyCodes,decorators:[{type:Inject,args:[\"K\"]}]},",
                );
                assert_contains(params, "{type:i2.KeyCodes,");
                assert_contains(params, "{type:i2.Renamed,");
            }
        }
    }

    #[test]
    fn namespace_import_of_only_const_enums_adds_no_import() {
        // ngtsc: no `import * as i1 from "./keys"`.
        let source = "import { Injectable, Inject } from '@angular/core';
import * as k from './keys';
@Injectable()
export class Svc {
  constructor(@Inject('K') a: k.KeyCodes, @Inject('D') d: k.Decl) {}
}";
        for mode in MODES {
            let code =
                compile_files(&[("keys.ts", NS_KEYS), ("a.ts", source)], &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"D\"]}]}]",
                "{mode:?}"
            );
            assert_not_contains(&code, "'./keys'");
        }
    }

    #[test]
    fn const_enum_in_an_imported_namespace_has_no_type() {
        // `k.NS.Inner` and `NS.Inner` of `import { NS }`. ngtsc: `undefined`
        // for the const enums, `i1.NS.InnerPlain` for the regular ones.
        let keys = "export const enum KeyCodes { A = 1, B = 2 }
export namespace NS { export const enum Inner { A = 1 } export enum InnerPlain { A = 1 } }
";
        let source = "import { Injectable, Inject } from '@angular/core';
import * as k from './keys';
import { NS } from './keys';
@Injectable()
export class Svc {
  constructor(@Inject('I') i: k.NS.Inner, @Inject('IP') ip: k.NS.InnerPlain, @Inject('N') n: NS.Inner, @Inject('NP') np: NS.InnerPlain) {}
}";
        for mode in MODES {
            let code = compile_files(&[("keys.ts", keys), ("a.ts", source)], &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"I\"]}]},\
                 {type:i1.NS.InnerPlain,decorators:[{type:Inject,args:[\"IP\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"N\"]}]},\
                 {type:i1.NS.InnerPlain,decorators:[{type:Inject,args:[\"NP\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn every_re_export_form_of_a_const_enum_has_no_type() {
        // `export { X } from`, renamed, `export *`, import then export,
        // `export * as kk` through `export *`, and `export type { X } from`.
        // ngtsc: `undefined` except the two regular enums.
        let source = "import { Injectable, Inject } from '@angular/core';
import { KeyCodes, Plain, Star, Renamed, LocalKC, kk, TypeKC } from './barrel';
@Injectable()
export class Svc {
  constructor(@Inject('KC') kc: KeyCodes, @Inject('P') p: Plain, @Inject('S') s: Star, @Inject('R') r: Renamed, @Inject('L') l: LocalKC, @Inject('KK') x: kk.KeyCodes, @Inject('KP') y: kk.Plain, @Inject('T') t: TypeKC) {}
}";
        let files = [
            ("keys.ts", "export const enum KeyCodes { A = 1 }\nexport enum Plain { X = 1 }\n"),
            ("star.ts", "export const enum Star { A = 1 }\n"),
            ("nsx.ts", "export * as kk from './keys';\n"),
            (
                "barrel.ts",
                "export { KeyCodes, Plain } from './keys';
export { KeyCodes as Renamed } from './keys';
export * from './star';
export * from './nsx';
import { KeyCodes as LocalKC } from './keys';
export { LocalKC };
export type { KeyCodes as TypeKC } from './keys';
",
            ),
            ("a.ts", source),
        ];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"KC\"]}]},\
                 {type:i1.Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"S\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"R\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"L\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"KK\"]}]},\
                 {type:i1.kk.Plain,decorators:[{type:Inject,args:[\"KP\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"T\"]}]}]",
                "{mode:?}"
            );
        }
    }

    const KK_FILES: [(&str, &str); 3] = [
        ("keys.ts", "export const enum KeyCodes { A = 1, B = 2 }\nexport enum Plain { X = 1 }\n"),
        ("barrel.ts", "export * as kk from './keys';\n"),
        (
            "a.ts",
            "import { Injectable, Inject } from '@angular/core';
import { kk } from './barrel';
import * as b from './barrel';
@Injectable()
export class Svc {
  constructor(@Inject('K') a: kk.KeyCodes, @Inject('P') p: kk.Plain, @Inject('B') c: b.kk.KeyCodes, @Inject('A') d: b.kk.KeyCodes.A) {}
}",
        ),
    ];

    #[test]
    fn const_enum_through_export_star_as_has_no_type() {
        // ngtsc: `undefined`, `i1.kk.Plain`, `undefined`, and the enum
        // member keeps `i1.kk.KeyCodes.A`.
        for mode in MODES {
            let code = compile_files(&KK_FILES, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:i1.kk.Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"B\"]}]},\
                 {type:i1.kk.KeyCodes.A,decorators:[{type:Inject,args:[\"A\"]}]}]",
                "{mode:?}"
            );
        }
    }

    /// Whether `dependencies` lists `file`.
    fn lists(dependencies: &[String], file: &str) -> bool {
        let file = std::fs::canonicalize(file).unwrap();
        dependencies.iter().any(|dep| std::fs::canonicalize(dep).is_ok_and(|dep| dep == file))
    }

    #[test]
    fn every_file_a_const_enum_lookup_reads_is_a_dependency() {
        // `b.kk.KeyCodes` reads barrel.ts then keys.ts; `c.Gone` reads
        // other.ts and finds nothing, which a later edit can change.
        let dir = TempDir::new().unwrap();
        let keys = write(&dir, "keys.ts", "export const enum KeyCodes { A = 1 }\n");
        let barrel = write(&dir, "barrel.ts", "export * as kk from './keys';\n");
        let other = write(&dir, "other.ts", "export const x = 1;\n");
        let source = "import { Injectable, Inject } from '@angular/core';
import * as b from './barrel';
import * as c from './other';
@Injectable()
export class Svc {
  constructor(@Inject('B') a: b.kk.KeyCodes, @Inject('G') g: c.Gone) {}
}";
        let path = write(&dir, "a.ts", source);
        let allocator = Allocator::default();
        let result = transform_angular_file(
            &allocator,
            &path,
            source,
            Some(&options(CompilationMode::Full, true)),
            None,
        );
        for file in [&keys, &barrel, &other] {
            assert!(lists(&result.dependencies, file), "{file}: {:?}", result.dependencies);
        }
    }

    const DTS: &str = "export declare const enum Typed { A = 1 }
export declare enum TypedPlain { A = 1 }
export declare namespace TNS { const enum E { A = 1 } }
declare const enum Hidden { A = 1 }
";

    const DTS_IMPORTER: &str = "import { Injectable, Inject } from '@angular/core';
import { Typed, TypedPlain, TNS, Hidden } from './typed';
import * as t from './typed';
@Injectable()
export class Svc {
  constructor(@Inject('T') a: Typed, @Inject('TP') b: TypedPlain, @Inject('NS') c: TNS.E, @Inject('H') d: Hidden, @Inject('NT') e: t.Typed) {}
}";

    #[test]
    fn const_enum_from_a_declaration_file_has_no_type() {
        // A `declare namespace` exports every member, and so does a `.d.ts`
        // file without `export {}`: `Hidden` is exported. ngtsc: `undefined`
        // except the regular enum.
        let dir = TempDir::new().unwrap();
        let typed = write(&dir, "typed.d.ts", DTS);
        let path = write(&dir, "a.ts", DTS_IMPORTER);
        for mode in MODES {
            let allocator = Allocator::default();
            let result = transform_angular_file(
                &allocator,
                &path,
                DTS_IMPORTER,
                Some(&options(mode, true)),
                None,
            );
            let code: String = result.code.chars().filter(|c| !c.is_whitespace()).collect();
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"T\"]}]},\
                 {type:i1.TypedPlain,decorators:[{type:Inject,args:[\"TP\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"NS\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"H\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"NT\"]}]}]",
                "{mode:?}"
            );
            assert!(lists(&result.dependencies, &typed), "{:?}", result.dependencies);
        }
    }

    #[test]
    fn const_enum_behind_a_js_specifier_has_no_type() {
        // TypeScript reads `./x.js` as x.ts or x.d.ts, `.mjs` as `.d.mts`,
        // `.cjs` as `.d.cts`, and prefers both.d.ts over both.js. The
        // specifier written stays. ngtsc: `undefined` except the regular enum.
        let source = "import { Injectable, Inject } from '@angular/core';
import { Typed, TypedPlain } from './typed.js';
import { KeyCodes } from './keys.js';
import { Mjs } from './m.mjs';
import { Cjs } from './c.cjs';
import { Both } from './both';
@Injectable()
export class Svc {
  constructor(@Inject('T') a: Typed, @Inject('TP') b: TypedPlain, @Inject('K') c: KeyCodes, @Inject('M') d: Mjs, @Inject('C') e: Cjs, @Inject('B') f: Both) {}
}";
        let files = [
            (
                "typed.d.ts",
                "export declare const enum Typed { A = 1 }\nexport declare enum TypedPlain { A = 1 }\n",
            ),
            ("keys.ts", "export const enum KeyCodes { A = 1 }\n"),
            ("m.d.mts", "export declare const enum Mjs { A = 1 }\n"),
            ("c.d.cts", "export declare const enum Cjs { A = 1 }\n"),
            ("both.js", "export const Both = {};\n"),
            ("both.d.ts", "export declare const enum Both { A = 1 }\n"),
            ("a.ts", source),
        ];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"T\"]}]},\
                 {type:i1.TypedPlain,decorators:[{type:Inject,args:[\"TP\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"M\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"C\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"B\"]}]}]",
                "{mode:?}"
            );
            assert_contains(&code, "import*asi1from'./typed.js'");
        }
    }

    #[test]
    fn const_enum_from_package_typings_has_no_type() {
        // The `types` export condition (with `./x.js` chunks read as
        // `.d.ts`, import then export, `export *`, a namespace), the
        // `types` and `typings` fields, and `main` read as `.d.ts`. ngtsc:
        // `undefined` except the two regular enums.
        let source = "import { Injectable, Inject } from '@angular/core';
import { Chunked, ChunkPlain, Renamed, Starred, Direct, DirectPlain, NS } from 'pkga';
import * as pa from 'pkga';
import { B1 } from 'pkgb';
import { C1 } from 'pkgc';
import { D1 } from 'pkgd';
@Injectable()
export class Svc {
  constructor(@Inject('1') a: Chunked, @Inject('2') b: ChunkPlain, @Inject('3') c: Renamed, @Inject('4') d: Starred, @Inject('5') e: Direct, @Inject('6') f: DirectPlain, @Inject('7') g: NS.Inner, @Inject('8') h: pa.Direct, @Inject('9') i: B1, @Inject('10') j: C1, @Inject('11') k: D1) {}
}";
        let files = [
            (
                "node_modules/pkga/package.json",
                r#"{ "name": "pkga", "version": "1.0.0", "type": "module", "exports": { ".": { "types": "./types/index.d.ts", "default": "./fesm/index.mjs" } } }"#,
            ),
            (
                "node_modules/pkga/types/index.d.ts",
                "import { Chunked } from './_chunk.js';
export { Chunked };
export { ChunkPlain, ChunkConst as Renamed } from './_chunk.js';
export * from './_star.js';
export declare const enum Direct { A = 1 }
export declare enum DirectPlain { A = 1 }
export declare namespace NS { const enum Inner { A = 1 } }
",
            ),
            (
                "node_modules/pkga/types/_chunk.d.ts",
                "declare const enum Chunked { A = 1 }
declare enum ChunkPlain { A = 1 }
declare const enum ChunkConst { A = 1 }
export { Chunked, ChunkPlain, ChunkConst };
",
            ),
            ("node_modules/pkga/types/_star.d.ts", "export declare const enum Starred { A = 1 }\n"),
            (
                "node_modules/pkgb/package.json",
                r#"{ "name": "pkgb", "version": "1.0.0", "main": "./dist/index.js", "types": "./dist/index.d.ts" }"#,
            ),
            ("node_modules/pkgb/dist/index.d.ts", "export declare const enum B1 { A = 1 }\n"),
            ("node_modules/pkgb/dist/index.js", "export {}\n"),
            (
                "node_modules/pkgc/package.json",
                r#"{ "name": "pkgc", "version": "1.0.0", "main": "./lib/index.js", "typings": "./lib/index.d.ts" }"#,
            ),
            ("node_modules/pkgc/lib/index.d.ts", "export declare const enum C1 { A = 1 }\n"),
            ("node_modules/pkgc/lib/index.js", "export {}\n"),
            (
                "node_modules/pkgd/package.json",
                r#"{ "name": "pkgd", "version": "1.0.0", "main": "./index.js" }"#,
            ),
            ("node_modules/pkgd/index.d.ts", "export declare const enum D1 { A = 1 }\n"),
            ("node_modules/pkgd/index.js", "export {}\n"),
            ("a.ts", source),
        ];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"1\"]}]},\
                 {type:i1.ChunkPlain,decorators:[{type:Inject,args:[\"2\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"3\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"4\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"5\"]}]},\
                 {type:i1.DirectPlain,decorators:[{type:Inject,args:[\"6\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"7\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"8\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"9\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"10\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"11\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn exported_merges_through_a_namespace_import_have_no_type() {
        // ngtsc: `undefined` four times, and no `./keys` import.
        let keys = "export const enum T1 { A = 1 }
export namespace T1 { export type X = 1; }
export namespace T2 { export interface I {} }
export const enum T2 { A = 1 }
export declare const enum D1 { A = 1 }
export declare namespace D1 { type X = 1; }
";
        let source = "import { Injectable, Inject } from '@angular/core';
import { T1, T2, D1 } from './keys';
import * as k from './keys';
@Injectable()
export class Svc {
  constructor(@Inject('T1') t1: T1, @Inject('T2') t2: T2, @Inject('D1') d1: D1, @Inject('KT1') kt1: k.T1) {}
}";
        for mode in MODES {
            let code = compile_files(&[("keys.ts", keys), ("a.ts", source)], &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"T1\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"T2\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"D1\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"KT1\"]}]}]",
                "{mode:?}"
            );
            assert_not_contains(&code, "'./keys'");
        }
    }

    #[test]
    fn export_star_as_adds_no_names_to_a_plain_star_walk() {
        // `export * as other` holds the const enum; the plain `export *`
        // gives the regular enum that the import reads, whatever the file
        // names or the order of the two lines. ngtsc: `i1.KeyCodes` and
        // `i1.kk.KeyCodes`.
        let importer = "import { Injectable, Inject } from '@angular/core';
import { KeyCodes } from './barrel';
@Injectable()
export class Svc { constructor(@Inject('K') a: KeyCodes) {} }";
        for (const_file, enum_file) in [("x", "mid"), ("zz", "aa")] {
            let barrel = format!(
                "export * as other from './{const_file}';\nexport * from './{enum_file}';\n"
            );
            let files = [
                (&*format!("{const_file}.ts"), "export const enum KeyCodes { A = 1 }\n"),
                (&*format!("{enum_file}.ts"), "export enum KeyCodes { A = 1 }\n"),
                ("barrel.ts", &*barrel),
                ("a.ts", importer),
            ];
            for mode in MODES {
                let code = compile_files(&files, &options(mode, true));
                assert_eq!(
                    ctor_params(&code),
                    "[{type:i1.KeyCodes,decorators:[{type:Inject,args:[\"K\"]}]}]",
                    "{mode:?}"
                );
                assert_contains(&code, "from'./barrel'");
            }
        }
        let files = [
            ("keys.ts", "export enum KeyCodes { A = 1 }\n"),
            ("keys2.ts", "export const enum KeyCodes { A = 1 }\n"),
            ("mid.ts", "export * as kk from './keys';\n"),
            ("x.ts", "export * as kk from './keys2';\n"),
            ("barrel.ts", "export * as other from './x';\nexport * from './mid';\n"),
            (
                "a.ts",
                "import { Injectable, Inject } from '@angular/core';
import * as b from './barrel';
@Injectable()
export class Svc { constructor(@Inject('K') a: b.kk.KeyCodes) {} }",
            ),
        ];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:i1.kk.KeyCodes,decorators:[{type:Inject,args:[\"K\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn default_exported_const_enum_has_no_type() {
        // `export { E as default }` and `export default E`, read by a
        // default import or `k.default`. ngtsc: `undefined` except
        // `i1.Plain`.
        let source = "import { Injectable, Inject } from '@angular/core';
import KD from './keys';
import * as k from './keys';
import ED from './exdef';
@Injectable()
export class Svc {
  constructor(@Inject('KD') a: KD, @Inject('K') b: k.default, @Inject('P') c: k.Plain, @Inject('ED') d: ED) {}
}";
        let files = [
            (
                "keys.ts",
                "const enum KeyCodes { A = 1 }\nexport { KeyCodes as default };\nexport enum Plain { X = 1 }\n",
            ),
            ("exdef.ts", "const enum E { A = 1 }\nexport default E;\n"),
            ("a.ts", source),
        ];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"KD\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:i1.Plain,decorators:[{type:Inject,args:[\"P\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"ED\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn default_import_of_a_const_enum_is_removed() {
        // Nothing reads the import once `type:` is `undefined`. ngtsc drops
        // it, and `./typed` has no JS file to load. A const enum through a
        // default namespace (`KD.Inner`) too.
        let typed = "declare const enum E { A = 1 }\nexport default E;\n";
        let ns_keys = "enum E { A = 1 }
namespace E { export const enum Inner { B = 2 } }
export default E;
";
        let cases = [
            ("keys.ts", "const enum KeyCodes { A = 1 }\nexport { KeyCodes as default };\n", "KD"),
            ("typed.d.ts", typed, "KD"),
            ("keys.ts", ns_keys, "KD.Inner"),
        ];
        for (file, content, param_type) in cases {
            let module = file.trim_end_matches(".d.ts").trim_end_matches(".ts");
            let source = format!(
                "import {{ Injectable, Inject }} from '@angular/core';
import KD from './{module}';
@Injectable()
export class Svc {{
  constructor(@Inject('KD') kd: {param_type}) {{}}
}}"
            );
            for mode in MODES {
                let code =
                    compile_files(&[(file, content), ("a.ts", &source)], &options(mode, true));
                assert_eq!(
                    ctor_params(&code),
                    "[{type:undefined,decorators:[{type:Inject,args:[\"KD\"]}]}]",
                    "{file} {mode:?}"
                );
                assert_not_contains(&code, "importKD");
            }
        }
    }

    #[test]
    fn default_import_of_a_const_enum_stays_while_the_output_names_it() {
        // Today's output, kept: the factory names it (no `@Inject`; ngtsc
        // writes `ɵɵinvalidFactory()` there), and so does a nested class's
        // metadata, which keeps `type: KD`.
        let keys = "const enum KeyCodes { A = 1 }\nexport { KeyCodes as default };\n";
        let factory = "import { Injectable } from '@angular/core';
import KD from './keys';
@Injectable()
export class Svc { constructor(kd: KD) {} }";
        let nested = "import { Injectable, Inject } from '@angular/core';
import KD from './keys';
@Injectable()
export class Svc { constructor(@Inject('KD') kd: KD) {} }
export function make() {
  @Injectable()
  class Nested { constructor(@Inject('KD') kd: KD) {} }
  return Nested;
}";
        for source in [factory, nested] {
            for mode in MODES {
                let code =
                    compile_files(&[("keys.ts", keys), ("a.ts", source)], &options(mode, true));
                assert_contains(&code, "importKDfrom'./keys'");
            }
        }
    }

    #[test]
    fn locally_type_exported_const_enum_has_no_type() {
        // `export type { KeyCodes }` of a local const enum. ngtsc:
        // `undefined`, `i1.Plain`, and the `./keys` import stays for it.
        let source = "import { Injectable, Inject } from '@angular/core';
import { KeyCodes, Plain } from './keys';
@Injectable()
export class Svc {
  constructor(@Inject('K') a: KeyCodes, @Inject('P') p: Plain) {}
}";
        let keys = "const enum KeyCodes { A = 1 }\nexport type { KeyCodes };\nexport enum Plain { X = 1 }\n";
        for mode in MODES {
            let code = compile_files(&[("keys.ts", keys), ("a.ts", source)], &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:i1.Plain,decorators:[{type:Inject,args:[\"P\"]}]}]",
                "{mode:?}"
            );
            assert_contains(&code, "import*asi1from'./keys'");
        }
    }

    #[test]
    fn const_enum_member_keeps_its_type() {
        // An enum member is no `ConstEnum` symbol. ngtsc: `Local.A`,
        // `i1.KeyCodes.A`, `i1.KeyCodes.A`.
        let source = "import { Injectable, Inject } from '@angular/core';
import * as k from './keys';
import { KeyCodes } from './keys';
const enum Local { A = 1 }
@Injectable()
export class Svc {
  constructor(@Inject('A') a: Local.A, @Inject('B') b: KeyCodes.A, @Inject('C') c: k.KeyCodes.A) {}
}";
        let files = [("keys.ts", "export const enum KeyCodes { A = 1 }\n"), ("a.ts", source)];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:Local.A,decorators:[{type:Inject,args:[\"A\"]}]},\
                 {type:i1.KeyCodes.A,decorators:[{type:Inject,args:[\"B\"]}]},\
                 {type:i1.KeyCodes.A,decorators:[{type:Inject,args:[\"C\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn type_only_namespace_import_stays_undefined() {
        // ngtsc: `undefined` for both, as before.
        let source = "import { Injectable, Inject } from '@angular/core';
import type * as k from './keys';
@Injectable()
export class Svc { constructor(@Inject('K') a: k.KeyCodes, @Inject('P') p: k.Plain) {} }";
        let files = [
            ("keys.ts", "export const enum KeyCodes { A = 1 }\nexport enum Plain { X = 1 }\n"),
            ("a.ts", source),
        ];
        for mode in MODES {
            let code = compile_files(&files, &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"K\"]}]},\
                 {type:undefined,decorators:[{type:Inject,args:[\"P\"]}]}]",
                "{mode:?}"
            );
        }
    }

    #[test]
    fn a_namespace_member_does_not_hide_an_exported_const_enum() {
        // A bare `namespace Inner` doesn't add to the file's exports.
        // ngtsc: `undefined`.
        let source = "import { Injectable, Inject } from '@angular/core';
import { Blurred } from './blurred';
@Injectable()
export class Svc {
  constructor(@Inject('B') b: Blurred) {}
}";
        let blurred = "export const enum Blurred { A = 1 }
namespace Inner { export class Blurred {} }
export const x = Inner;
";
        for mode in MODES {
            let code =
                compile_files(&[("blurred.ts", blurred), ("a.ts", source)], &options(mode, true));
            assert_eq!(
                ctor_params(&code),
                "[{type:undefined,decorators:[{type:Inject,args:[\"B\"]}]}]",
                "{mode:?}"
            );
        }
    }
}

#[test]
fn a_const_enum_kept_inside_one_namespace_block_does_not_hide_an_exported_class() {
    // ngtsc 22.2.3: `type: NS.E`. Only the class is `NS.E` from outside.
    let source = "import { Injectable } from '@angular/core';
export namespace NS { const enum E { A = 1 } export const x = E.A; }
export namespace NS { export class E {} }
@Injectable()
export class Svc { constructor(e: NS.E) {} }
";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(ctor_params(&code), "[{type:NS.E}]", "{mode:?}");
    }
}

#[test]
fn an_exported_namespace_const_enum_is_still_undefined() {
    let source = "import { Injectable } from '@angular/core';
export namespace NS { export const enum E { A = 1 } }
declare namespace Amb { const enum F { A = 1 } }
@Injectable()
export class Svc { constructor(e: NS.E, f: Amb.F) {} }
";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(ctor_params(&code), "[{type:undefined},{type:undefined}]", "{mode:?}");
    }
}

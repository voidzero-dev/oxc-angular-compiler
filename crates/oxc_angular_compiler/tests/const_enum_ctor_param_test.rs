//! A constructor parameter typed by a `const enum` gets `type: undefined` in
//! the class metadata (full `ɵsetClassMetadata`, partial
//! `ɵɵngDeclareClassMetadata`), like ngtsc's `typeToValue` outside local
//! compilation (issue #623). Each expected output was checked against
//! `@angular/compiler-cli` 22.2.3 in full and partial mode.
//!
//! - same file: `const enum`, `declare const enum`, `ns.E`
//! - imported: only with `resolve_imported_values`, when the other file can
//!   be read and shows a `const enum`
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
fn const_enum_merged_with_a_namespace_keeps_its_type() {
    // ngtsc gives `undefined` here too, but one file can't be sure what the
    // namespace merges with, so the reference stays.
    let source = "import { Injectable, Inject } from '@angular/core';
const enum M { A = 1 }
namespace M { export type T = 1; }
@Injectable()
export class Svc { constructor(@Inject('M') m: M) {} }";
    for mode in MODES {
        let code = compile(source, mode);
        assert_eq!(
            ctor_params(&code),
            "[{type:M,decorators:[{type:Inject,args:[\"M\"]}]}]",
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
    fn unsure_imported_const_enum_keeps_its_type() {
        // A namespace import (ngtsc: `undefined`), a file whose bare
        // `namespace` block blurs its export names, a `.d.ts`-only module
        // and a missing one: oxc can't be sure, so `i{n}.X` stays.
        let dir = TempDir::new().unwrap();
        write(&dir, "keys.ts", "export const enum KeyCodes { A = 1 }\n");
        write(
            &dir,
            "blurred.ts",
            "export const enum Blurred { A = 1 }\nnamespace Inner { export class Blurred {} }\n",
        );
        write(&dir, "dts.d.ts", "export declare const enum Typed { A = 1 }\n");
        let source = "import { Injectable, Inject } from '@angular/core';
import * as k from './keys';
import { Blurred } from './blurred';
import { Typed } from './dts';
import { Missing } from './missing';
@Injectable()
export class Svc {
  constructor(@Inject('K') a: k.KeyCodes, @Inject('B') b: Blurred, @Inject('T') t: Typed, @Inject('M') m: Missing) {}
}";
        let path = write(&dir, "a.ts", source);
        for mode in MODES {
            let code = compile_at(&path, source, &options(mode, true));
            let params = ctor_params(&code);
            assert_contains(params, "{type:i1.KeyCodes,");
            assert_contains(params, ".Blurred,");
            assert_contains(params, ".Typed,");
            assert_contains(params, ".Missing,");
            assert_not_contains(params, "type:undefined");
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

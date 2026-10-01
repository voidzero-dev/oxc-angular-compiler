//! `static ngAcceptInputType_<input>: T;` in the `.d.ts`: `T` is the input
//! transform's first parameter type, printed the way ngtsc prints it (its
//! `TypeEmitter` plus TypeScript's printer).
//!
//! Every expected string below is what `@angular/compiler-cli` 22.1.7 wrote
//! into the `.d.ts` for [`SOURCE`] with the type in place of `%TYPE%`.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

const SOURCE: &str = "import {Directive, Input, Signal, ElementRef, booleanAttribute, Signal as S} from '@angular/core';
import type {TemplateRef, WritableSignal} from '@angular/core';
import * as ng from '@angular/core';
import type {ToSignalOptions} from '@angular/core/rxjs-interop';
import {Other} from './other';
import * as oth from './other';
export type TT = 'a' | 'b';
export interface Box { b: 1 }
export enum Color { Red, Green }
export const enum CE { A = 'a' }
export interface Gen<T> { v: T }
export const val = {a: 1};
export const kc = 'kc';
export function gen<T>(x: T) { return x; }
@Directive({selector: '[t]'})
export class T {
  @Input({transform: (v: %TYPE%) => 1}) t: any;
}
";

/// The type oxc writes for `ngAcceptInputType_t` when the transform's
/// parameter has type `ty`.
fn accept_type(ty: &str) -> String {
    let source = SOURCE.replace("%TYPE%", ty);
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "t.ts",
        &source,
        Some(&TransformOptions::default()),
        None,
    );
    let decl = result
        .dts_declarations
        .iter()
        .find(|d| d.class_name == "T")
        .unwrap_or_else(|| panic!("no .d.ts declaration for `{ty}`"));
    let marker = "static ngAcceptInputType_t: ";
    let at = decl
        .members
        .find(marker)
        .unwrap_or_else(|| panic!("no ngAcceptInputType_t for `{ty}`:\n{}", decl.members));
    let rest = &decl.members[at + marker.len()..];
    rest.trim_end().strip_suffix(';').unwrap_or(rest).to_string()
}

fn check(cases: &[(&str, &str)]) {
    let failures: Vec<String> = cases
        .iter()
        .filter_map(|(ty, expected)| {
            let actual = accept_type(ty);
            (actual != *expected)
                .then(|| format!("  {ty:?}\n    expected {expected:?}\n    actual   {actual:?}"))
        })
        .collect();
    assert!(failures.is_empty(), "{} mismatches:\n{}", failures.len(), failures.join("\n"));
}

#[test]
fn matches_ngtsc() {
    check(MATCH);
}

/// Comments ngtsc keeps that oxc drops: `/** */` comments after a type or
/// on their own line (TypeScript's printer keeps only those two kinds of
/// comment around nodes), and comments spanning lines. Each row is the type,
/// what ngtsc writes and what oxc writes; they differ only by the comment.
const COMMENTS_NOT_KEPT: &[(&str, &str, &str)] = &[
    ("{\n  /** doc */\n  a: 1\n}", "{ \n    /** doc */\n    a: 1; }", "{ a: 1; }"),
    ("string | /* a\n b */ number", "string | /* a\n   b */ number", "string | number"),
    ("string /** d */ | number", "string /** d */ | number", "string | number"),
    ("{ a: 1 /** y */ }", "{ a: 1; /** y */ }", "{ a: 1; }"),
    ("(x: string /** y */) => void", "(x: string /** y */) => void", "(x: string) => void"),
    ("Array<string /** y */>", "Array<string /** y */>", "Array<string>"),
    ("string |\n /** j */ number", "string | \n    /** j */ number", "string | number"),
    // TypeScript reads past a U+2028 / U+2029 for the comments after `<` (it
    // only stops at `\n` and `\r`), and prints this one twice.
    (
        "Array<\u{2028}/* c */\nstring | number>",
        "Array</* c */ /* c */ string | number>",
        "Array<string | number>",
    ),
    (
        "Array<\u{2029}/* c */\nstring | number>",
        "Array</* c */ /* c */ string | number>",
        "Array<string | number>",
    ),
];

#[test]
fn comments_not_kept() {
    // Without comments and whitespace, ngtsc's text is oxc's.
    let bare = |text: &str| {
        let mut out = String::new();
        let mut rest = text;
        while let Some(at) = rest.find("/*") {
            out.push_str(&rest[..at]);
            rest = &rest[at + rest[at..].find("*/").unwrap() + 2..];
        }
        out.push_str(rest);
        out.split_whitespace().collect::<String>()
    };
    for (ty, ngtsc, oxc) in COMMENTS_NOT_KEPT {
        assert_eq!(bare(ngtsc), bare(oxc), "{ty:?}");
    }
    let cases: Vec<(&str, &str)> =
        COMMENTS_NOT_KEPT.iter().map(|(ty, _, oxc)| (*ty, *oxc)).collect();
    check(&cases);
}

/// ngtsc adds `import * as i1 from './other'` and writes `i1.Other`; oxc
/// writes `unknown` for any type that names another module (see `dts_type.rs`).
#[test]
fn other_module_types_are_unknown() {
    let cases: Vec<(&str, &str)> = OTHER_MODULE.iter().map(|(ty, _)| (*ty, "unknown")).collect();
    check(&cases);
}

/// ngtsc throws "Unable to emit import type" and aborts the whole build.
#[test]
fn import_types_are_unknown() {
    check(&[("import('./other').Other", "unknown"), ("typeof import('./other')", "unknown")]);
}

/// An enum member (`Color.Red`): ngtsc 22.1.7 throws while emitting the
/// `.d.ts` ("Unsupported WrappedNodeExpr in TypeTranslatorVisitor:
/// EnumMember"), so there's nothing to match. oxc keeps the type as written
/// rather than shortening it to `Red` like other qualified names the file
/// declares (`NS.T` is written `T`, see the snapshot's `probe: dts-qualified*`).
#[test]
fn enum_member_type_is_kept_as_written() {
    check(&[("Color.Red", "Color.Red"), ("Color.Red | Color.Green", "Color.Red | Color.Green")]);
}

/// The same for a member of an enum declared in a namespace (`NS.E.A`, also
/// through a class merged with the namespace, nested namespaces and
/// `namespace A.B`): ngtsc 22.1.7 throws there too. A qualified name that
/// isn't an enum member is still shortened like ngtsc does (`NS.E` is `E`,
/// `A.B.T` is `T`).
#[test]
fn nested_enum_member_type_is_kept_as_written() {
    let source = "import {Directive, Input} from '@angular/core';
export namespace NS { export enum E { A = 'a' } export namespace M { export enum E { B = 'b' } } }
export class C {}
export namespace C { export enum E { A = 'a' } }
export namespace A.B { export enum E { X = 'x' } export type T = string; }
@Directive({selector: '[d]'})
export class Dir {
  @Input({transform: (v: NS.E.A | C.E.A | NS.M.E.B | A.B.E.X) => 1}) x!: number;
  @Input({transform: (v: NS.E | NS.M.E | A.B.T) => 1}) y!: number;
}
";
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "t.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    let decl = result.dts_declarations.iter().find(|d| d.class_name == "Dir").unwrap();
    assert!(
        decl.members.contains("static ngAcceptInputType_x: NS.E.A | C.E.A | NS.M.E.B | A.B.E.X;"),
        "{}",
        decl.members
    );
    assert!(decl.members.contains("static ngAcceptInputType_y: E | E | T;"), "{}", decl.members);
}

/// The same whatever the member's name is written as: `'B'`, `['C']` or a
/// template literal (`` [`A`] ``, also with an escape), at the top level or in
/// a namespace. ngtsc 22.1.7 throws on all of these.
#[test]
fn enum_members_named_by_literals_are_kept_as_written() {
    let source = "import {Directive, Input} from '@angular/core';
export enum E { [`A`] = 'a', 'B' = 'b', ['C'] = 'c', [`\\u0044`] = 'd' }
export namespace NS { export enum F { [`A`] = 'a' } }
@Directive({selector: '[d]'})
export class Dir {
  @Input({transform: (v: E.A | E.B | E.C | E.D) => 1}) x!: number;
  @Input({transform: (v: NS.F.A) => 1}) y!: number;
}
";
    assert_eq!(
        accept_members(source),
        vec![
            "static ngAcceptInputType_x: E.A | E.B | E.C | E.D;".to_string(),
            "static ngAcceptInputType_y: NS.F.A;".to_string(),
        ]
    );
}

/// A transform declared in a namespace (reached through `typeof NS.fn`, a
/// static method of a class there, or `import H = NS.fn`) is typed from its
/// declaration, like ngtsc 22.1.7 types this source. Its parameter's types
/// are printed as written: ngtsc writes the namespace's `T` as `T` too, which
/// doesn't resolve in the `.d.ts` (the snapshot skips these, since ngtsc also
/// emits `fn` for `F`, which isn't in scope; oxc keeps `F`).
#[test]
fn transforms_declared_in_namespaces_are_typed_like_ngtsc() {
    let source = "import {Directive, Input} from '@angular/core';
export namespace NS {
  export type T = string;
  export function fn(v: string) { return 1; }
  export function g(v: T) { return 1; }
  export class C { static s(v: number) { return 1; } }
}
declare const F: typeof NS.fn;
declare const G: typeof NS.g;
declare const K: typeof NS.C;
import H = NS.fn;
@Directive({selector: '[d]'})
export class Dir {
  @Input({transform: F}) x0!: number;
  @Input({transform: G}) x1!: number;
  @Input({transform: K.s}) x2!: number;
  @Input({transform: H}) x3!: number;
}
";
    assert_eq!(accept_members(source), expect_members(&["string", "T", "number", "string"]));
}

/// The `ngAcceptInputType_*` members oxc writes for `source`, in order.
fn accept_members(source: &str) -> Vec<String> {
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "t.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    let decl = result.dts_declarations.iter().find(|d| d.class_name == "Dir").unwrap();
    decl.members
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with("static ngAcceptInputType_"))
        .map(str::to_string)
        .collect()
}

/// `x0`, `x1`, ... typed with `types`, one member each.
fn members_typed(types: &[&str]) -> String {
    types
        .iter()
        .enumerate()
        .map(|(i, t)| format!("  @Input({{transform: (v: {t}) => 1}}) x{i}!: number;\n"))
        .collect()
}

fn expect_members(types: &[&str]) -> Vec<String> {
    types.iter().enumerate().map(|(i, t)| format!("static ngAcceptInputType_x{i}: {t};")).collect()
}

/// An `import A = NS` / `import C = NS.T` alias (`export import` too) is
/// written as the declaration it resolves to: ngtsc 22.1.7 wrote each
/// expected type for this source. A `typeof` stays as written, like any other.
#[test]
fn import_equals_aliases_are_typed_like_their_target() {
    let types = [
        "A.T",
        "B.U",
        "C",
        "Array<C>",
        "D.T",
        "E",
        "A.In.U",
        "F.U",
        "G",
        "typeof A.x",
        "{ [k: string]: C }",
    ];
    let source = format!(
        "import {{Directive, Input}} from '@angular/core';
export namespace NS {{ export interface T {{ a: string }} export namespace In {{ export interface U {{ a: string }} }} export const x = 1; }}
import A = NS;
import B = NS.In;
import C = NS.T;
export import D = NS;
export import E = NS.T;
import F = A.In;
import G = B.U;
@Directive({{selector: '[d]'}})
export class Dir {{
{}}}
",
        members_typed(&types)
    );
    assert_eq!(
        accept_members(&source),
        expect_members(&[
            "T",
            "U",
            "T",
            "Array<T>",
            "T",
            "T",
            "U",
            "U",
            "U",
            "typeof A.x",
            "{ [k: string]: T; }"
        ])
    );
}

/// A chain of aliases is followed to its end however long it is: ngtsc 22.1.7
/// writes `T` for `A17.T` through 18 aliases, and `T | number` through 40. A
/// cycle (a TypeScript error, on which ngtsc throws) gives `unknown`.
#[test]
fn import_equals_alias_chains_are_followed_to_the_end() {
    let chain =
        |n: usize| -> String { (1..n).map(|i| format!("import A{i} = A{};\n", i - 1)).collect() };
    let types = ["A17.T", "A39.T | number", "X.T", "S.T"];
    let source = format!(
        "import {{Directive, Input}} from '@angular/core';
namespace NS {{ export type T = string; }}
import A0 = NS;
{}import X = Y;
import Y = X;
import S = S.B;
@Directive({{selector: '[d]'}})
export class Dir {{
{}}}
",
        chain(40),
        members_typed(&types)
    );
    assert_eq!(accept_members(&source), expect_members(&["T", "T | number", "unknown", "unknown"]));
}

/// An alias of another module (`import R = require('./other')`, or of an
/// import) follows the rule for other modules: `unknown`. ngtsc 22.1.7 writes
/// the bare name there (`Other`, `T`), which doesn't resolve in its `.d.ts`.
/// An alias of `@angular/core`, through a namespace import or
/// `import Core = require('@angular/core')`, becomes `i0.X` (ngtsc: the bare
/// `Signal<number>`, equally unresolved; it throws on a `require` declared in
/// a namespace, which TypeScript also rejects), and one of a global is written
/// as its target (`Intl.NumberFormat`, where ngtsc writes `NumberFormat`).
#[test]
fn import_equals_aliases_of_other_modules_and_globals() {
    let cases: [(&str, &[&str], &[&str]); 5] = [
        (
            "import R = require('./other');\n",
            &["R.Other", "R.Inner.T", "typeof R.FLAG", "R.Other | string"],
            &["unknown", "unknown", "typeof R.FLAG", "unknown"],
        ),
        (
            "import * as o from './other';\nimport {Inner} from './other';\nimport N = o.Inner;\nimport M = Inner;\nimport K = o;\n",
            &["N.T", "M.T", "K.Other", "K.Inner.T"],
            &["unknown", "unknown", "unknown", "unknown"],
        ),
        (
            "import * as ng from '@angular/core';\nimport S = ng.Signal;\nimport Core = ng;\n",
            &["S<number>", "Core.Signal<string>", "Core.ElementRef"],
            &["i0.Signal<number>", "i0.Signal<string>", "i0.ElementRef"],
        ),
        (
            "import Core = require('@angular/core');\nimport S = Core.Signal;\nimport C2 = Core;\nexport import Ex = require('@angular/core');\nnamespace NS { export import S2 = Core.Signal; export import C3 = Core; }\nnamespace NR { export import Core2 = require('@angular/core'); }\n",
            &[
                "Core.Signal<string>",
                "Core.ElementRef | null",
                "S<number>",
                "C2.Signal<string>",
                "Ex.ElementRef",
                "NS.S2<string>",
                "NS.C3.ElementRef",
                "NR.Core2.Signal<string>",
                "typeof Core.booleanAttribute",
            ],
            &[
                "i0.Signal<string>",
                "i0.ElementRef | null",
                "i0.Signal<number>",
                "i0.Signal<string>",
                "i0.ElementRef",
                "i0.Signal<string>",
                "i0.ElementRef",
                "i0.Signal<string>",
                "typeof Core.booleanAttribute",
            ],
        ),
        (
            "import NF = Intl.NumberFormat;\nimport I = Intl;\n",
            &["NF", "I.NumberFormat"],
            &["Intl.NumberFormat", "Intl.NumberFormat"],
        ),
    ];
    for (imports, types, expected) in cases {
        let source = format!(
            "import {{Directive, Input}} from '@angular/core';
{imports}@Directive({{selector: '[d]'}})
export class Dir {{
{}}}
",
            members_typed(types)
        );
        assert_eq!(accept_members(&source), expect_members(expected), "{imports}");
    }
}

/// Declarations for [`namespace_member_aliases_are_typed_like_their_target`]
/// and [`namespace_member_aliases_of_other_modules_and_globals`]: import-equals
/// aliases declared in namespaces, of names that namespace, an enclosing one
/// or the file declares.
const NAMESPACE_ALIASES: &str = "import * as ng from '@angular/core';
import * as oth from './other';
export class Local { a = 1; }
export namespace Local { export interface M { m: 1 } }
export function Fn() {}
export namespace Fn { export interface Z { z: 1 } }
export enum En { A }
export namespace En { export interface Q { q: 1 } }
export namespace Other { export interface T { c: 1 } export namespace Deep { export interface U { d: 1 } } }
export namespace NS {
  export import Alias = Local;
  export namespace Inner { export import Alias2 = Local; export import Up = Alias; }
  export import A1 = Local;
  export import A2 = A1;
  export import OT = Other.T;
  export import O = Other;
  export import OD = Other.Deep;
  export class In2 { i = 1; }
  export namespace In2 { export interface W { w: 1 } }
  export import Rel = In2;
  export import FA = Fn;
  export import EA = En;
  export import Core = ng;
  export import CoreE = ng.ElementRef;
  export import Oth = oth;
  export import OthT = oth.FooNs.T;
  export import NF = Intl.NumberFormat;
  export import G = Inner.Alias2;
  export import N2 = NS2;
}
export namespace NS2 { export import B = NS.Alias; export interface V { v: 1 } }
export class Local3 { b = 1; }
export namespace Local3 { export import Self = Local3; export import MM = Local.M; }
import X = NS.Alias;
import Y = NS.Inner;
";

/// `accept_members` for [`NAMESPACE_ALIASES`] with `types`.
fn namespace_alias_members(types: &[&str]) -> Vec<String> {
    accept_members(&format!(
        "import {{Directive, Input}} from '@angular/core';
{NAMESPACE_ALIASES}@Directive({{selector: '[d]'}})
export class Dir {{
{}}}
",
        members_typed(types)
    ))
}

/// An alias declared in a namespace (`NS.Alias`, `NS.Inner.Alias2`, an alias
/// of an alias, or one whose target is named from the namespace's scope) is
/// written as the declaration it resolves to: ngtsc 22.1.7 wrote each expected
/// type for [`NAMESPACE_ALIASES`], one type per compile.
#[test]
fn namespace_member_aliases_are_typed_like_their_target() {
    let cases = [
        ("NS.Alias", "Local"),
        ("NS.Alias.M", "M"),
        ("NS.Inner.Alias2", "Local"),
        ("NS.Inner.Alias2.M", "M"),
        ("NS.Inner.Up", "Local"),
        ("NS.A2", "Local"),
        ("NS.A2.M", "M"),
        ("NS.OT", "T"),
        ("NS.O.T", "T"),
        ("NS.O.Deep.U", "U"),
        ("NS.OD.U", "U"),
        ("NS.Rel", "In2"),
        ("NS.Rel.W", "W"),
        ("NS.In2", "In2"),
        ("NS.FA.Z", "Z"),
        ("NS.EA", "En"),
        ("NS.EA.Q", "Q"),
        ("NS.G", "Local"),
        ("NS.N2.V", "V"),
        ("NS.N2.B", "Local"),
        ("NS2.B", "Local"),
        ("Local3.Self", "Local3"),
        ("Local3.MM", "M"),
        ("X", "Local"),
        ("X.M", "M"),
        ("Y.Alias2", "Local"),
        ("Y.Up", "Local"),
        ("Array<NS.Alias>", "Array<Local>"),
        ("typeof NS.Alias", "typeof NS.Alias"),
        ("NS.Alias | NS.A2", "Local | Local"),
        ("{ k: NS.Inner.Alias2 }", "{ k: Local; }"),
    ];
    let types: Vec<&str> = cases.iter().map(|(ty, _)| *ty).collect();
    let expected: Vec<&str> = cases.iter().map(|(_, expected)| *expected).collect();
    assert_eq!(namespace_alias_members(&types), expect_members(&expected));
}

/// A namespace member alias of another module, `@angular/core` or a global
/// follows the rules for a top-level alias (see
/// `import_equals_aliases_of_other_modules_and_globals`). ngtsc 22.1.7 writes
/// the bare `ElementRef`, `Foo`, `T` (none of which resolve in its `.d.ts`)
/// and `NumberFormat`. It throws on the enum member `NS.EA.A`, which oxc
/// writes as the member it resolves to, like `E.A`.
#[test]
fn namespace_member_aliases_of_other_modules_and_globals() {
    let cases = [
        ("NS.Core.ElementRef", "i0.ElementRef"),
        ("NS.CoreE", "i0.ElementRef"),
        ("NS.Oth.Foo", "unknown"),
        ("NS.OthT", "unknown"),
        ("NS.NF", "Intl.NumberFormat"),
        ("NS.EA.A", "En.A"),
    ];
    let types: Vec<&str> = cases.iter().map(|(ty, _)| *ty).collect();
    let expected: Vec<&str> = cases.iter().map(|(_, expected)| *expected).collect();
    assert_eq!(namespace_alias_members(&types), expect_members(&expected));
}

/// A transform that's a global declared outside the file (the DOM's `atob`, a
/// project's `declare function`) is assumed to be a function but can't be
/// inspected, so its type is `unknown`, like an imported transform's. ngtsc
/// 22.1.7 reads the lib and writes `string` for `atob` and `any` for `alert`.
/// A function of TypeScript's ES lib (`parseInt`) is typed.
#[test]
fn global_transforms_declared_outside_the_file_are_unknown() {
    let source = "import {Directive, Input} from '@angular/core';
const t = atob;
@Directive({selector: '[d]', inputs: [{name: 'x0', transform: alert}]})
export class Dir {
  x0!: string;
  @Input({transform: atob}) x1!: string;
  @Input({transform: t}) x2!: string;
  @Input({transform: parseInt}) x3!: number;
}
";
    assert_eq!(
        accept_members(source),
        expect_members(&["unknown", "unknown", "unknown", "string"])
    );
}

/// A name in a namespace merged with an enum (`E.T`) is shortened like any
/// other qualified local name, as ngtsc 22.1.7 does: only an actual enum
/// member (`E.A`, also from a second declaration of the enum) is kept as
/// written.
#[test]
fn names_in_a_namespace_merged_with_an_enum_are_shortened() {
    let types = ["E.T", "E.U", "E.N.V", "NS.F.T", "E.T | E.U", "E.A", "E.B", "NS.F.X"];
    let source = format!(
        "import {{Directive, Input}} from '@angular/core';
export enum E {{ A }}
export enum E {{ B = 1 }}
export namespace E {{ export interface T {{ a: string }} export type U = string; export namespace N {{ export interface V {{ a: 1 }} }} }}
export namespace NS {{ export enum F {{ X }} export namespace F {{ export interface T {{ a: 1 }} }} }}
@Directive({{selector: '[d]'}})
export class Dir {{
{}}}
",
        members_typed(&types)
    );
    assert_eq!(
        accept_members(&source),
        expect_members(&["T", "U", "V", "T", "T | U", "E.A", "E.B", "NS.F.X"])
    );
}

/// A computed property name follows the rules of a type name (see
/// `dts_type.rs`): ngtsc 22.1.7 copies the expression as written, so an
/// imported name (`{ [token]: 1; }`) doesn't resolve in its `.d.ts`. oxc
/// writes `@angular/core` names through `i0` and makes a type naming another
/// module `unknown`. Local and global names are written as is, like ngtsc
/// (`{ [kc]: 1 }` and `[Symbol.iterator]` in [`MATCH`]).
#[test]
fn computed_names_from_imports_follow_type_name_rules() {
    check(&[
        ("{ [Other]: 1 }", "unknown"),
        ("{ [oth.token]: 1 }", "unknown"),
        ("{ [oth.a.token](): void }", "unknown"),
        ("(x: { [Other]: 1 }) => void", "unknown"),
        ("{ [booleanAttribute]: 1 }", "{ [i0.booleanAttribute]: 1; }"),
        ("{ [ng.ɵSIGNAL]: 1 }", "{ [i0.ɵSIGNAL]: 1; }"),
    ]);
}

/// A computed property name through an import-equals alias (top level, in a
/// namespace, or of an alias) resolves it the way a type name does. ngtsc
/// 22.1.7 copies every one of these as written. An alias of a name the file
/// declares stays as written, as ngtsc wrote it (the last group). The rest
/// follow oxc's rules for imports and globals, since type-checking ngtsc's
/// `.d.ts` reports "Cannot find name" for each (except `Ex`, which is
/// exported): an alias of `@angular/core` gives `i0.X`, one of another module
/// makes the type `unknown`, and one of a global (`G`, declared in a separate
/// `.d.ts` for ngtsc) is written as its target.
#[test]
fn computed_names_through_import_equals_aliases() {
    let cases: [(&str, &[&str], &[&str]); 4] = [
        (
            "import Core = require('@angular/core');\nimport S = Core.ɵSIGNAL;\nimport C2 = Core;\nexport import Ex = require('@angular/core');\nimport * as ng from '@angular/core';\nimport C = ng;\nimport S2 = ng.ɵSIGNAL;\nnamespace W { export import C3 = Core; export import S3 = Core.ɵSIGNAL; }\n",
            &[
                "{ [Core.ɵSIGNAL]: string }",
                "{ [Core.ɵSIGNAL](): void }",
                "{ [S]: string }",
                "{ [C2.ɵSIGNAL]: string }",
                "{ [Ex.ɵSIGNAL]: string }",
                "{ [C.ɵSIGNAL]: string }",
                "{ [S2]: string }",
                "{ [W.C3.ɵSIGNAL]: string }",
                "{ [W.S3]: string }",
            ],
            &[
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL](): void; }",
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL]: string; }",
                "{ [i0.ɵSIGNAL]: string; }",
            ],
        ),
        (
            "import R = require('./other');\nimport * as o from './other';\nimport O = o;\nimport T = o.token;\nimport {Inner} from './other';\nimport I = Inner;\nnamespace W { export import O2 = o; }\n",
            &[
                "{ [R.token]: string }",
                "{ [R.Inner.k]: string }",
                "{ [O.token]: string }",
                "{ [T]: string }",
                "{ [I.k]: string }",
                "{ [W.O2.token]: string }",
            ],
            &["unknown", "unknown", "unknown", "unknown", "unknown", "unknown"],
        ),
        (
            "import GA = G;\nimport GK = G.k;\nimport GI = G.In;\nnamespace W { export import GA2 = G; }\n",
            &[
                "{ [GA.k]: string }",
                "{ [GK]: string }",
                "{ [GI.j]: string }",
                "{ [W.GA2.k]: string }",
            ],
            &[
                "{ [G.k]: string; }",
                "{ [G.k]: string; }",
                "{ [G.In.j]: string; }",
                "{ [G.k]: string; }",
            ],
        ),
        (
            "namespace NS { export const k = 'nk'; }\nimport A = NS;\nimport K = NS.k;\nnamespace W { export import A2 = NS; }\n",
            &["{ [A.k]: string }", "{ [K]: string }", "{ [W.A2.k]: string }"],
            &["{ [A.k]: string; }", "{ [K]: string; }", "{ [W.A2.k]: string; }"],
        ),
    ];
    for (imports, types, expected) in cases {
        let source = format!(
            "import {{Directive, Input}} from '@angular/core';
{imports}@Directive({{selector: '[d]'}})
export class Dir {{
{}}}
",
            members_typed(types)
        );
        assert_eq!(accept_members(&source), expect_members(expected), "{imports}");
    }
}

/// An overloaded static method is typed from its first declaration, as an
/// overloaded function is (ngtsc 22.1.7 writes `string` here). The snapshot
/// can't hold this case: ngtsc compiles the transform to the bare method name
/// `c`, which oxc deliberately doesn't.
#[test]
fn overloaded_static_method_is_typed_from_its_first_declaration() {
    let source = "import {Directive, Input} from '@angular/core';
export class U { static c(v: string): number; static c(v: string | number) { return 1; } }
@Directive({selector: '[d]'})
export class Dir {
  @Input({transform: U.c}) x!: number;
}
";
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "t.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    let decl = result.dts_declarations.iter().find(|d| d.class_name == "Dir").unwrap();
    assert!(decl.members.contains("static ngAcceptInputType_x: string;"), "{}", decl.members);
}

/// The type in the transform, and what ngtsc writes for it.
const MATCH: &[(&str, &str)] = &[
    ("{[K in 'a']: Signal<number>}", "{ [K in \"a\"]: i0.Signal<number>; }"),
    ("{[K in TT]: Signal<number>}", "{ [K in TT]: i0.Signal<number>; }"),
    ("{readonly [K in TT]?: Signal<number>}", "{ readonly [K in TT]?: i0.Signal<number>; }"),
    ("{-readonly [K in TT]-?: Signal<number>}", "{ -readonly [K in TT]-?: i0.Signal<number>; }"),
    ("{+readonly [K in TT]+?: Signal<number>}", "{ +readonly [K in TT]+?: i0.Signal<number>; }"),
    ("{[K in TT as 'z']: Signal<number>}", "{ [K in TT as \"z\"]: i0.Signal<number>; }"),
    ("{[K in keyof Box]: Signal<number>}", "{ [K in keyof Box]: i0.Signal<number>; }"),
    ("{[K in TT]: Signal<number>} | null", "{ [K in TT]: i0.Signal<number>; } | null"),
    ("{[K in TT]}", "{ [K in TT]: ; }"),
    ("{[K in TT]: Signal<number>;}", "{ [K in TT]: i0.Signal<number>; }"),
    ("{ m(): Signal<number> }", "{ m(): i0.Signal<number>; }"),
    ("{ m?(): Signal<number> }", "{ m?(): i0.Signal<number>; }"),
    (
        "{ m(a: Signal<number>, b?: string, ...c: number[]): void }",
        "{ m(a: i0.Signal<number>, b?: string, ...c: number[]): void; }",
    ),
    ("{ m(this: Signal<number>): void }", "{ m(this: i0.Signal<number>): void; }"),
    ("{ m<T extends Signal<number>>(): void }", "{ m<T extends i0.Signal<number>>(): void; }"),
    ("{ 'n'(): void; ['c'](): void; 5(): void }", "{ \"n\"(): void; [\"c\"](): void; 5(): void; }"),
    ("{ [k: string]: Signal<number> }", "{ [k: string]: i0.Signal<number>; }"),
    ("{ readonly [k: string]: Signal<number> }", "{ readonly [k: string]: i0.Signal<number>; }"),
    ("{ (x: Signal<number>): void }", "{ (x: i0.Signal<number>): void; }"),
    ("{ <T extends Signal<number>>(): void }", "{ <T extends i0.Signal<number>>(): void; }"),
    ("{ new (x: ElementRef): void }", "{ new (x: i0.ElementRef): void; }"),
    ("{ new <T extends Signal<number>>(): Box }", "{ new <T extends i0.Signal<number>>(): Box; }"),
    (
        "{ get x(): Signal<number>; set x(v: Signal<number>) }",
        "{ get x(): i0.Signal<number>; set x(v: i0.Signal<number>); }",
    ),
    ("{ get 'g'(): Signal<number> }", "{ get \"g\"(): i0.Signal<number>; }"),
    ("{ m(): void, n: 1 }", "{ m(): void; n: 1; }"),
    ("{ m() }", "{ m(); }"),
    ("{ a }", "{ a; }"),
    ("{ readonly r?: Signal<number> }", "{ readonly r?: i0.Signal<number>; }"),
    ("(x: unknown) => x is Signal<number>", "(x: unknown) => x is i0.Signal<number>"),
    (
        "(x: unknown) => asserts x is Signal<number>",
        "(x: unknown) => asserts x is i0.Signal<number>",
    ),
    ("(x: unknown) => asserts x", "(x: unknown) => asserts x"),
    ("<T extends Signal<number>>() => void", "<T extends i0.Signal<number>>() => void"),
    ("<T = Signal<number>>() => void", "<T = i0.Signal<number>>() => void"),
    (
        "<T extends Signal<number> = Signal<number>, U = string>() => void",
        "<T extends i0.Signal<number> = i0.Signal<number>, U = string>() => void",
    ),
    ("<const T extends string>() => void", "<const T extends string>() => void"),
    ("(this: Signal<number>) => void", "(this: i0.Signal<number>) => void"),
    ("(this: Signal<number>, a: string) => void", "(this: i0.Signal<number>, a: string) => void"),
    ("new <T extends Signal<number>>() => Box", "new <T extends i0.Signal<number>>() => Box"),
    ("abstract new () => Signal<number>", "abstract new () => i0.Signal<number>"),
    ("({w}: {w: 3}) => void", "({ w }: { w: 3; }) => void"),
    ("({w: ww, ...rest}: {w: 3}) => void", "({ w: ww, ...rest }: { w: 3; }) => void"),
    ("([a, , b]: [1, 2, 3]) => void", "([a, , b]: [1, 2, 3]) => void"),
    ("([a, ...r]: [1, 2]) => void", "([a, ...r]: [1, 2]) => void"),
    ("({a: {b}}: {a: {b: 1}}) => void", "({ a: { b } }: { a: { b: 1; }; }) => void"),
    ("({}: {}) => void", "({}: {}) => void"),
    ("([]: []) => void", "([]: []) => void"),
    ("({'q-x': qx}: {'q-x': 1}) => void", "({ \"q-x\": qx }: { \"q-x\": 1; }) => void"),
    ("({['c']: cc}: {c: 1}) => void", "({ [\"c\"]: cc }: { c: 1; }) => void"),
    ("(x?: Signal<number>) => void", "(x?: i0.Signal<number>) => void"),
    ("(x) => void", "(x) => void"),
    ("(...xs) => void", "(...xs) => void"),
    ("typeof this", "typeof this"),
    ("{ a: typeof this }", "{ a: typeof this; }"),
    ("typeof this | string", "typeof this | string"),
    ("Array<typeof this>", "Array<typeof this>"),
    ("'é'", "\"\\u00E9\""),
    ("'😀'", "\"\\uD83D\\uDE00\""),
    ("'\\0'", "\"\\0\""),
    ("'\\x00'", "\"\\0\""),
    ("'\\0' | '\\u00001'", "\"\\0\" | \"\\x001\""),
    ("'\u{2028}\u{2029}\\u0085'", "\"\\u2028\\u2029\\u0085\""),
    // A U+2028 / U+2029 (a line break, but three bytes long) on the line
    // before a union.
    ("{ a: 1;\u{2028} b:\n string | number }", "{ a: 1; b: string | number; }"),
    ("{ a: 1;\u{2029} b:\n string | number }", "{ a: 1; b: string | number; }"),
    ("'\\v\\f\\b'", "\"\\v\\f\\b\""),
    ("'\\x7f'", "\"\u{7f}\""),
    ("'a\"b\\'c`d'", "\"a\\\"b'c`d\""),
    ("'\\r\\n'", "\"\\r\\n\""),
    ("'\\u{10FFFF}'", "\"\\uDBFF\\uDFFF\""),
    ("'\\uD800'", "\"\\uD800\""),
    ("'\\uDC00x'", "\"\\uDC00x\""),
    ("'a\\uD800b\\uDFFF'", "\"a\\uD800b\\uDFFF\""),
    ("'\\u{FFFD}'", "\"\\uFFFD\""),
    ("'\\uFFFD\\uD800'", "\"\\uFFFD\\uD800\""),
    ("{ '\\uD800': 1 }", "{ \"\\uD800\": 1; }"),
    ("{ 'x\\uDBFF': 1; '\\uD800\\uDC00': 2 }", "{ \"x\\uDBFF\": 1; \"\\uD800\\uDC00\": 2; }"),
    (
        "({'\\uD800': a}: {'\\uD800': 1}) => void",
        "({ \"\\uD800\": a }: { \"\\uD800\": 1; }) => void",
    ),
    ("`é`", "`é`"),
    ("`é${string}`", "`é${string}`"),
    ("`aé${number}b`", "`aé${number}b`"),
    (
        "{ 'q-x': 1; \"dq\": 2; 3: 4; 0x10: 5; 1.50: 6; 1e3: 7 }",
        "{ \"q-x\": 1; \"dq\": 2; 3: 4; 16: 5; 1.5: 6; 1000: 7; }",
    ),
    ("{ 'é': 1 }", "{ \"\\u00E9\": 1; }"),
    ("{ é: 1 }", "{ é: 1; }"),
    ("{ 'a\\nb': 1 }", "{ \"a\\nb\": 1; }"),
    ("{ ['lit']: 1 }", "{ [\"lit\"]: 1; }"),
    ("{ [kc]: 1 }", "{ [kc]: 1; }"),
    ("{ [Symbol.iterator]: Signal<number> }", "{ [Symbol.iterator]: i0.Signal<number>; }"),
    ("{ m(): void; 'n'?: 2 }", "{ m(): void; \"n\"?: 2; }"),
    (
        "1e21 | 0.1 | -1 | 0b101 | 0o17 | 1_000 | 0x1Fn | 123456789012345678901234567890",
        "1e+21 | 0.1 | -1 | 5 | 15 | 1000 | 0x1fn | 1.2345678901234568e+29",
    ),
    ("- 1", "-1"),
    ("typeof val", "typeof val"),
    ("typeof val.a", "typeof val.a"),
    ("typeof gen<Signal<number>>", "typeof gen<i0.Signal<number>>"),
    ("Signal<number>['set']", "i0.Signal<number>[\"set\"]"),
    ("TT extends `${infer _}` ? 1 : 2", "TT extends `${infer _}` ? 1 : 2"),
    (
        "string extends infer U extends Signal<number> ? 1 : 2",
        "string extends infer U extends i0.Signal<number> ? 1 : 2",
    ),
    ("readonly Signal<number>[]", "readonly i0.Signal<number>[]"),
    ("Array<{ m(): Signal<number> }>", "Array<{ m(): i0.Signal<number>; }>"),
    ("[Signal<number>?, ...Signal<string>[]]", "[i0.Signal<number>?, ...i0.Signal<string>[]]"),
    ("/* c */ string", "string"),
    ("string /* d */", "string"),
    ("/* c */ string /* d */ | number", "/* c */ string | number"),
    ("string | /* e */ number", "string | /* e */ number"),
    ("Array</* x */ string>", "Array</* x */ string>"),
    ("{ /* x */ a: 1 }", "{ /* x */ a: 1; }"),
    ("{ a: 1 /* y */ }", "{ a: 1; }"),
    ("// line\n  string", "string"),
    ("/** doc */ string", "string"),
    ("{ /** doc */ a: 1 }", "{ /** doc */ a: 1; }"),
    ("(/* p */ x: string) => void", "(/* p */ x: string) => void"),
    ("Signal<number> & { a: 1 }", "i0.Signal<number> & { a: 1; }"),
    ("keyof Signal<number>", "keyof i0.Signal<number>"),
    ("unique symbol", "unique symbol"),
    ("{ a: 1; /* z */ b: 2 }", "{ a: 1; /* z */ b: 2; }"),
    ("{ a: 1; // z\n b: 2 }", "{ a: 1; // z\n    b: 2; }"),
    ("string | // z\n number", "string | // z\n    number"),
    ("/* a */ /* b */ string | number", "/* a */ /* b */ string | number"),
    ("Array<string /* x */>", "Array<string>"),
    ("{ a: /* x */ 1 }", "{ a: 1; }"),
    ("{ a: /* x */ Signal<number> }", "{ a: i0.Signal<number>; }"),
    ("{ a: /* x */ Box }", "{ a: Box; }"),
    ("(x: string, /* y */ y: number) => void", "(x: string, /* y */ y: number) => void"),
    ("(x: string) => /* r */ void", "(x: string) => void"),
    ("/* a */ (string)", "(string)"),
    ("(/* a */ string)", "(string)"),
    ("{ /* x */ }", "{}"),
    ("[/* a */ string]", "[/* a */ string]"),
    ("[string /* a */, number]", "[string, number]"),
    ("keyof /* k */ Box", "keyof Box"),
    ("/* a */\n string | number", "/* a */ string | number"),
    ("{\n  // line\n  a: 1;\n  b: 2\n}", "{ a: 1; b: 2; }"),
    ("Signal</* x */ number>", "i0.Signal</* x */ number>"),
    ("/* c */ Signal<number> | null", "/* c */ i0.Signal<number> | null"),
    ("/* c */ Array<string>", "Array<string>"),
    ("/* c */ string[]", "string[]"),
    ("/* c */ { a: 1 }", "{ a: 1; }"),
    ("/* c */ (x: string) => void", "(x: string) => void"),
    ("/* c */ 'lit' | 1", "/* c */ \"lit\" | 1"),
    ("/* c */ Box | null", "/* c */ Box | null"),
    ("/* c */ Box", "Box"),
    ("string[] /* c */ | number", "string[] | number"),
    ("string /* d */ & number", "string & number"),
    ("(string /* d */)", "(string)"),
    ("{ a: 1 /* y */; b: 2 }", "{ a: 1; b: 2; }"),
    ("{ a: 1; /* y */ }", "{ a: 1; }"),
    (
        "0XABn | 0b101n | 0o17n | 1_000n | 0x1_Fn | 10n | 0Xabn",
        "0xabn | 5n | 15n | 1000n | 0x1fn | 10n | 0xabn",
    ),
    ("{ 0x10(): void; 1.50?: 1; get 1e3(): 1 }", "{ 16(): void; 1.5?: 1; get 1000(): 1; }"),
    ("{ m(): this is Signal<number> }", "{ m(): this is i0.Signal<number>; }"),
    ("{ m(): asserts this }", "{ m(): asserts this; }"),
    ("[...rest: Signal<number>[]]", "[...rest: i0.Signal<number>[]]"),
    ("[a?: Signal<number>]", "[a?: i0.Signal<number>]"),
    ("?string", "?string"),
    ("string?", "?string"),
    (
        "TT extends 'a' ? TT extends 'b' ? 1 : 2 : 3",
        "TT extends \"a\" ? TT extends \"b\" ? 1 : 2 : 3",
    ),
    ("{ [K in TT]?: Signal<number> }", "{ [K in TT]?: i0.Signal<number>; }"),
    ("(x: string) => (y: number) => void", "(x: string) => (y: number) => void"),
    ("() => void", "() => void"),
    ("new () => Box", "new () => Box"),
    ("Signal<Signal<number>>", "i0.Signal<i0.Signal<number>>"),
    ("Box[][]", "Box[][]"),
    ("keyof typeof val", "keyof typeof val"),
    ("Array</* x */ string | number>", "Array</* x */ /* x */ string | number>"),
    ("(/* x */ string | number)[]", "(/* x */ string | number)[]"),
    ("{ a: /* x */ string | number }", "{ a: /* x */ string | number; }"),
    ("[/* x */ a: string]", "[/* x */ a: string]"),
    ("<T, /* x */ U>() => void", "<T, /* x */ U>() => void"),
    ("</* x */ T>() => void", "</* x */ T>() => void"),
    ("{ m(/* x */ a: string): void }", "{ m(/* x */ a: string): void; }"),
    ("({ /* x */ w }: { w: 1 }) => void", "({ /* x */ w }: { w: 1; }) => void"),
    ("([/* x */ a]: [1]) => void", "([/* x */ a]: [1]) => void"),
    ("new (/* x */ x: string) => Box", "new (/* x */ x: string) => Box"),
    ("[/* a */ ...string[]]", "[/* a */ ...string[]]"),
    ("/* x */ | 'a' | 'b'", "\"a\" | \"b\""),
    ("| /* x */ 'a' | 'b'", "/* x */ \"a\" | \"b\""),
    ("{ a: 1, /* x */ b: 2 }", "{ a: 1; /* x */ b: 2; }"),
    ("{ a: 1 /* x */ ; b: 2 }", "{ a: 1; b: 2; }"),
    ("{ a: 1; /* x */\n b: 2 }", "{ a: 1; /* x */ b: 2; }"),
    ("string |\n /* x */ number", "string | number"),
    ("string | /* x */\n number", "string | /* x */ number"),
    ("Signal</* x */ number, /* y */ string>", "i0.Signal</* x */ number, /* y */ string>"),
    ("{ /* x */ [k: string]: 1 }", "{ /* x */ [k: string]: 1; }"),
    ("{ /* x */ (): void }", "{ /* x */ (): void; }"),
    ("{ /* x */ [K in TT]: 1 }", "{ [K in TT]: 1; }"),
    ("(/* a */ /* b */ x: string) => void", "(/* a */ /* b */ x: string) => void"),
    ("string | /* a */ /* b */ number", "string | /* a */ /* b */ number"),
    ("(/* x */ this: Box) => void", "(/* x */ this: Box) => void"),
    ("{ /* x */ get g(): 1 }", "{ /* x */ get g(): 1; }"),
    ("/** r */\n string | number", "/** r */ string | number"),
    ("/* x */ /** j */ string | number", "/* x */ /** j */ string | number"),
    ("string /* a */ | /* b */ number", "string | /* b */ number"),
    ("!string", "!string"),
    ("string!", "!string"),
    ("?", "?"),
    ("S<number>", "i0.Signal<number>"),
    ("ng.Signal<string>", "i0.Signal<string>"),
    ("TemplateRef<unknown>", "i0.TemplateRef<unknown>"),
    ("WritableSignal<number>", "i0.WritableSignal<number>"),
    ("typeof booleanAttribute", "typeof booleanAttribute"),
    ("typeof oth", "typeof oth"),
    ("Color", "Color"),
    ("CE", "CE"),
    ("Gen<string>['v']", "Gen<string>[\"v\"]"),
    ("Parameters<typeof booleanAttribute>[0]", "Parameters<typeof booleanAttribute>[0]"),
    ("{a: string\n       b: number}", "{ a: string; b: number; }"),
    ("\n     | 'a'\n     | 'b'", "\"a\" | \"b\""),
    ("'q\"u\\'o\\\\té\\n😀'", "\"q\\\"u'o\\\\t\\u00E9\\n\\uD83D\\uDE00\""),
    ("0x1F | 1e3 | 1_000 | .5 | -0 | 10n | -10n", "31 | 1000 | 1000 | 0.5 | -0 | 10n | -10n"),
    ("1.50 | 0.1e-7 | 1e21 | 0b101 | 0o17", "1.5 | 1e-8 | 1e+21 | 5 | 15"),
    (
        "'tab\\there' | \"a'b\" | 'uni\\u{1F600}' | '\\x41' | '\\0'",
        "\"tab\\there\" | \"a'b\" | \"uni\\uD83D\\uDE00\" | \"A\" | \"\\0\"",
    ),
    ("{\"quoted-key\": 1; 'single': 2; 3: 4}", "{ \"quoted-key\": 1; \"single\": 2; 3: 4; }"),
    ("{a: 1,}", "{ a: 1; }"),
    ("Uppercase<TT>", "Uppercase<TT>"),
    (
        "Date | HTMLElement | Map<string, Set<number>>",
        "Date | HTMLElement | Map<string, Set<number>>",
    ),
    ("{ a: 1 /* x */\n b: 2 }", "{ a: 1; /* x */ b: 2; }"),
    ("{ a: 1 // x\n b: 2 }", "{ a: 1; // x\n    b: 2; }"),
    ("Array</* a */ string | /* b */ number>", "Array</* a */ /* a */ string | /* b */ number>"),
    ("- 0x10 | -0b11n", "-16 | -3n"),
    ("{ [/* x */ k: string]: 1 }", "{ [/* x */ k: string]: 1; }"),
    ("(x: string) => (/* y */ y: number) => void", "(x: string) => (/* y */ y: number) => void"),
    ("`${ /* x */ string}`", "`${string}`"),
    ("{ 'a': 1 }\n", "{ \"a\": 1; }"),
    ("string\n | number", "string | number"),
    ("(x: string, ) => void", "(x: string) => void"),
    ("<T,>() => void", "<T>() => void"),
    ("typeof val . a", "typeof val.a"),
    ("TT extends 'a' ? 1 : 2", "TT extends \"a\" ? 1 : 2"),
    ("`x-${TT}`", "`x-${TT}`"),
    (
        "(a: string, b?: number, ...c: boolean[]) => void",
        "(a: string, b?: number, ...c: boolean[]) => void",
    ),
    ("new (x: number) => Box", "new (x: number) => Box"),
    ("abstract new () => Box", "abstract new () => Box"),
    ("[a: string, b?: number, ...c: boolean[]]", "[a: string, b?: number, ...c: boolean[]]"),
    ("[string?, ...number[]]", "[string?, ...number[]]"),
    ("readonly [string, number]", "readonly [string, number]"),
    ("ReadonlyArray<string>", "ReadonlyArray<string>"),
    ("{[k: string]: number}", "{ [k: string]: number; }"),
    (
        "{(x: number): string; new (): Box; m(): void; readonly r: 1; 'q-x': 2; [Symbol.iterator]: 3}",
        "{ (x: number): string; new (): Box; m(): void; readonly r: 1; \"q-x\": 2; [Symbol.iterator]: 3; }",
    ),
    ("((string))", "((string))"),
    ("(string | number)[][]", "(string | number)[][]"),
    ("(() => void) | null", "(() => void) | null"),
    ("keyof TT[]", "keyof TT[]"),
    ("`${number}px`", "`${number}px`"),
    ("`a\\`b\\n${string}`", "`a\\`b\\n${string}`"),
    ("{a: string, b: number}", "{ a: string; b: number; }"),
    ("{get x(): string; set x(v: string)}", "{ get x(): string; set x(v: string); }"),
    ("(x: unknown) => x is string", "(x: unknown) => x is string"),
    ("Array<string|number>", "Array<string | number>"),
    ("{a:{b:{c:1}}}", "{ a: { b: { c: 1; }; }; }"),
    (
        "(x: {a: 1}, [y, z]: [1, 2], {w}: {w: 3}) => void",
        "(x: { a: 1; }, [y, z]: [1, 2], { w }: { w: 3; }) => void",
    ),
    ("string = 'x'", "string"),
    ("{ get x(): Signal<number> }", "{ get x(): i0.Signal<number>; }"),
    ("{ 'a b': Signal<number> }", "{ \"a b\": i0.Signal<number>; }"),
    ("[a: Signal<number>]", "[a: i0.Signal<number>]"),
    ("(...a: Signal<number>[]) => void", "(...a: i0.Signal<number>[]) => void"),
    ("(a?: Signal<number>) => void", "(a?: i0.Signal<number>) => void"),
];

/// Types naming `./other`, and what ngtsc writes for them.
const OTHER_MODULE: &[(&str, &str)] = &[
    ("{[K in 'a']: Other}", "{ [K in \"a\"]: i1.Other; }"),
    ("{ m(): Other }", "{ m(): i1.Other; }"),
    ("{ [k: number]: Other }", "{ [k: number]: i1.Other; }"),
    ("{ set x(v: Other) }", "{ set x(v: i1.Other); }"),
    ("{ get x(): Other }", "{ get x(): i1.Other; }"),
    ("(x: unknown) => x is Other", "(x: unknown) => x is i1.Other"),
    ("(x: unknown) => asserts x is Other", "(x: unknown) => asserts x is i1.Other"),
    ("<T extends Other>() => void", "<T extends i1.Other>() => void"),
    ("(this: Other) => void", "(this: i1.Other) => void"),
    ("new (x: Other) => Box", "new (x: i1.Other) => Box"),
    ("typeof gen<Other>", "typeof gen<i1.Other>"),
    ("[a: Signal<number>, b?: Other]", "[a: i0.Signal<number>, b?: i1.Other]"),
    ("(Signal<number> | Other)[]", "(i0.Signal<number> | i1.Other)[]"),
    ("oth.Other", "i1.Other"),
    ("ng.ElementRef<ng.Signal<oth.Other>>", "i0.ElementRef<i0.Signal<i1.Other>>"),
    ("ToSignalOptions<number>", "i1.ToSignalOptions<number>"),
    ("Other", "i1.Other"),
    ("{[K in TT]: Other}", "{ [K in TT]: i1.Other; }"),
];

/// The member name for inputs whose class property is a string key. ngtsc
/// quotes `ngAcceptInputType_<name>` only when the name has a `-` or `.`
/// (Angular's `isUnsafeObjectKey`), escaped the way TypeScript prints a
/// string literal; any other name is written as is, even when that doesn't
/// parse. Every expected line is what ngtsc 22.1.7 wrote for this source.
#[test]
fn member_name_is_quoted_like_ngtsc() {
    let source = r#"import {Directive, Input} from '@angular/core';
function tr(v: string | number): string { return String(v); }
@Directive({selector: '[m]'})
export class M {
  @Input({transform: tr}) 'a"b': any;
  @Input({transform: tr}) 'c\\d': any;
  @Input({transform: tr}) 'e\nf': any;
  @Input({transform: tr}) "i'j": any;
  @Input({transform: tr}) 'mé': any;
  @Input({transform: tr}) 'k-l': any;
  @Input({transform: tr}) 'a-"b': any;
  @Input({transform: tr}) 'c.\\d': any;
  @Input({transform: tr}) 'e-\nf': any;
  @Input({transform: tr}) 'k-é': any;
  @Input({transform: tr}) 'x-😀\u0001\u000b\0': any;
  @Input({transform: tr}) 'a.b': any;
  @Input({transform: tr}) "i-'j": any;
  @Input({transform: tr}) norm: any;
}
@Directive({selector: '[a]', inputs: [{name: 'q-"r', transform: tr}, {name: 's"t', transform: tr}]})
export class A { 'q-"r': any; 's"t': any; }
"#;
    let allocator = Allocator::default();
    let result = transform_angular_file(
        &allocator,
        "m.ts",
        source,
        Some(&TransformOptions::default()),
        None,
    );
    let members = |class: &str| {
        let decl = result.dts_declarations.iter().find(|d| d.class_name == class).unwrap();
        decl.members
            .split("\nstatic ")
            .filter(|m| {
                m.starts_with("ngAcceptInputType_") || m.starts_with("\"ngAcceptInputType_")
            })
            .map(|m| format!("static {m}"))
            .collect::<Vec<_>>()
    };
    let t = ": string | number;";
    let expect =
        |names: &[&str]| names.iter().map(|n| format!("static {n}{t}")).collect::<Vec<_>>();
    assert_eq!(
        members("M"),
        expect(&[
            "ngAcceptInputType_a\"b",
            "ngAcceptInputType_c\\d",
            "ngAcceptInputType_e\nf",
            "ngAcceptInputType_i'j",
            "ngAcceptInputType_mé",
            "\"ngAcceptInputType_k-l\"",
            "\"ngAcceptInputType_a-\\\"b\"",
            "\"ngAcceptInputType_c.\\\\d\"",
            "\"ngAcceptInputType_e-\\nf\"",
            "\"ngAcceptInputType_k-\\u00E9\"",
            "\"ngAcceptInputType_x-\\uD83D\\uDE00\\u0001\\v\\0\"",
            "\"ngAcceptInputType_a.b\"",
            "\"ngAcceptInputType_i-'j\"",
            "ngAcceptInputType_norm",
        ])
    );
    assert_eq!(members("A"), expect(&["\"ngAcceptInputType_q-\\\"r\"", "ngAcceptInputType_s\"t"]));
}

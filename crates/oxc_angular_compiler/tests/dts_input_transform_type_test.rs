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
    ("'é'", "\"\\u00E9\""),
    ("'😀'", "\"\\uD83D\\uDE00\""),
    ("'\\0'", "\"\\0\""),
    ("'\\x00'", "\"\\0\""),
    ("'\\0' | '\\u00001'", "\"\\0\" | \"\\x001\""),
    ("'\u{2028}\u{2029}\\u0085'", "\"\\u2028\\u2029\\u0085\""),
    ("'\\v\\f\\b'", "\"\\v\\f\\b\""),
    ("'\\x7f'", "\"\u{7f}\""),
    ("'a\"b\\'c`d'", "\"a\\\"b'c`d\""),
    ("'\\r\\n'", "\"\\r\\n\""),
    ("'\\u{10FFFF}'", "\"\\uDBFF\\uDFFF\""),
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

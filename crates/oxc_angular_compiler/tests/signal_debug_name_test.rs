//! ngtsc's implicit signal `debugName` transform
//! (`implicit_signal_debug_name_transform.ts`): signal APIs imported from
//! `@angular/core` (`httpResource` from `@angular/common`) get an ngDevMode
//! `debugName` argument named after the binding they initialize, in every
//! file. Expected texts are ngtsc 22.2.3's output, printed on one line.

use oxc_allocator::Allocator;
use oxc_angular_compiler::{TransformOptions, transform_angular_file};

fn transform(source: &str) -> String {
    transform_with(source, &TransformOptions::default())
}

fn transform_with(source: &str, options: &TransformOptions) -> String {
    let allocator = Allocator::default();
    let result = transform_angular_file(&allocator, "test.ts", source, Some(options), None);
    assert!(!result.has_errors(), "unexpected errors: {:?}", result.diagnostics);
    result.code
}

fn spread(name: &str) -> String {
    format!(
        "/* @ts-ignore */ ...(ngDevMode ? [{{ debugName: \"{name}\" }}] : /* istanbul ignore next */ [])"
    )
}

fn object_spread(name: &str) -> String {
    format!("...(ngDevMode ? {{ debugName: \"{name}\" }} : /* istanbul ignore next */ {{}})")
}

#[test]
fn component_fields_get_debug_names() {
    let code = transform(
        r"
import { Component, signal, computed, input, model, linkedSignal, viewChild, viewChildren, contentChild, effect, output, resource } from '@angular/core';
@Component({ selector: 'app-x', template: '<div #ref></div>' })
export class X {
  count = signal(0);
  dbl = computed(() => this.count() * 2);
  name = input('a');
  req = input.required<string>();
  aliased = input(0, { alias: 'al' });
  val = model(1);
  ls = linkedSignal(() => this.count());
  ref = viewChild('ref');
  refs = viewChildren('ref');
  cc = contentChild('cc');
  r = resource({ loader: async () => 1 });
  withOpts = signal(0, { equal: (a, b) => a === b });
  ev = effect(() => {});
  out = output();
}
",
    );
    for expected in [
        format!("count = signal(0, {})", spread("count")),
        format!("dbl = computed(() => this.count() * 2, {})", spread("dbl")),
        format!("name = input('a', {})", spread("name")),
        // `input.required` takes its options first.
        format!("req = input.required<string>({})", spread("req")),
        format!("aliased = input(0, {{ {}, alias: 'al' }})", object_spread("aliased")),
        format!("val = model(1, {})", spread("val")),
        format!("ls = linkedSignal(() => this.count(), {})", spread("ls")),
        format!("ref = viewChild('ref', {})", spread("ref")),
        format!("refs = viewChildren('ref', {})", spread("refs")),
        format!("cc = contentChild('cc', {})", spread("cc")),
        // `resource` takes only an options object.
        format!("r = resource({{ {}, loader: async () => 1 }})", object_spread("r")),
        format!(
            "withOpts = signal(0, {{ {}, equal: (a, b) => a === b }})",
            object_spread("withOpts")
        ),
        format!("ev = effect(() => {{}}, {})", spread("ev")),
    ] {
        assert!(code.contains(&expected), "expected `{expected}` in:\n{code}");
    }
    // `output` isn't a signal function.
    assert!(code.contains("out = output();"), "{code}");
}

#[test]
fn plain_file_without_angular_decorator() {
    let code = transform(
        r"
import { signal, computed as c } from '@angular/core';
export const top = signal(1);
const al = c(() => 1);
export class Plain {
  s = signal<number | undefined>(undefined);
  t: any;
  constructor() {
    this.t = signal(2);
  }
}
",
    );
    assert!(code.contains(&format!("export const top = signal(1, {});", spread("top"))), "{code}");
    // An alias isn't one of the names the transform looks for.
    assert!(code.contains("const al = c(() => 1);"), "{code}");
    assert!(
        code.contains(&format!("s = signal<number | undefined>(undefined, {});", spread("s"))),
        "{code}"
    );
    assert!(code.contains(&format!("this.t = signal(2, {});", spread("t"))), "{code}");
}

#[test]
fn argument_position_and_existing_options() {
    let code = transform(
        r"
import { Directive, signal, computed, input, model, linkedSignal, effect } from '@angular/core';
import { httpResource } from '@angular/common/http';
export const a = signal(0, { equal: undefined, debugName: 'mine' });
export const b = signal(0, { 'debugName': 'q' });
const opts = {};
export const d = signal(0, opts);
export const g = linkedSignal({ source: a, computation: () => 1 });
export const h = linkedSignal(() => 1, { equal: Object.is });
export const j = httpResource(() => '/x');
export const k = effect(() => {}, {});
export const w = signal(0,);
@Directive({ selector: '[d]' })
export class D {
  e = input.required({ alias: 'xx' });
  f = model.required<number>();
  x = input();
}
",
    );
    for expected in [
        // An options object that sets `debugName` itself is left alone.
        "export const a = signal(0, { equal: undefined, debugName: 'mine' });".to_string(),
        // ...but only an identifier key counts.
        format!("export const b = signal(0, {{ {}, 'debugName': 'q' }});", object_spread("b")),
        // Options that aren't an object literal are left alone.
        "export const d = signal(0, opts);".to_string(),
        format!(
            "export const g = linkedSignal({{ {}, source: a, computation: () => 1 }});",
            object_spread("g")
        ),
        format!("export const h = linkedSignal(() => 1, {{ {}, equal: Object.is }});", object_spread("h")),
        format!("export const j = httpResource(() => '/x', {});", spread("j")),
        format!("export const k = effect(() => {{}}, {{ {} }});", object_spread("k")),
        format!("export const w = signal(0, {},);", spread("w")),
        format!("e = input.required({{ {}, alias: 'xx' }});", object_spread("e")),
        format!("f = model.required<number>({});", spread("f")),
        // No arguments: the initial value is `undefined`.
        "x = input(/* @ts-ignore */ ...(ngDevMode ? [undefined, { debugName: \"x\" }] : /* istanbul ignore next */ []));".to_string(),
    ] {
        assert!(code.contains(&expected), "expected `{expected}` in:\n{code}");
    }
}

#[test]
fn names_and_positions() {
    let code = transform(
        r"
import { signal, computed } from '@angular/core';
export const l = computed(() => {
  const inner = signal(1);
  return inner();
});
function wrap(fn: any) { return fn(); }
export const m = wrap(() => { const inner2 = signal(2); return inner2; });
export let o = signal<number>(0), p = computed(() => 1);
export const t = (signal(0));
export const u = signal(0) as any;
export const bb = signal.bind(null);
function local() { const signal = (x: number) => x; const shadow = signal(1); return shadow; }
export class K {
  #priv = signal(0);
  'quoted-name' = signal(1);
  static st = signal(2);
  accessor acc = signal(3);
  ['comp'] = signal(4);
  other: any;
  constructor() {
    this.#priv = signal(6);
    (this as any).dyn = computed(() => 7);
    (this as any)['el'] = signal(8);
    let q = this.other = signal(9);
  }
}
for (const ff = signal(0); ;) { break; }
",
    );
    for expected in [
        // A transformed initializer's own body isn't visited.
        format!("  const inner = signal(1);\n  return inner();\n}}, {});", spread("l")),
        // Any other initializer is.
        format!("const inner2 = signal(2, {});", spread("inner2")),
        format!(
            "export let o = signal<number>(0, {}), p = computed(() => 1, {});",
            spread("o"),
            spread("p")
        ),
        "export const t = (signal(0));".to_string(),
        "export const u = signal(0) as any;".to_string(),
        format!("export const bb = signal.bind(null, {});", spread("bb")),
        // A shadowing local isn't Angular's `signal`.
        "const shadow = signal(1);".to_string(),
        format!("#priv = signal(0, {});", spread("#priv")),
        format!("'quoted-name' = signal(1, {});", spread("'quoted-name'")),
        format!("static st = signal(2, {});", spread("st")),
        format!("accessor acc = signal(3, {});", spread("acc")),
        format!("['comp'] = signal(4, {});", spread("['comp']")),
        format!("this.#priv = signal(6, {});", spread("#priv")),
        format!("(this as any).dyn = computed(() => 7, {});", spread("dyn")),
        "(this as any)['el'] = signal(8);".to_string(),
        "let q = this.other = signal(9);".to_string(),
        format!("for (const ff = signal(0, {}); ;)", spread("ff")),
    ] {
        assert!(code.contains(&expected), "expected `{expected}` in:\n{code}");
    }
}

#[test]
fn only_angular_imports() {
    let code = transform(
        r"
import { signal } from './my-signals';
import { computed } from '@angular/core/primitives/signals';
import { input } from '@angular/corex';
import * as ng from '@angular/core';
export const a = signal(0);
export const b = computed(() => 1);
export const c = input();
export const d = ng.signal(0);
",
    );
    assert!(code.contains("export const a = signal(0);"), "{code}");
    // An entry point of `@angular/core` is still Angular's.
    assert!(
        code.contains(&format!("export const b = computed(() => 1, {});", spread("b"))),
        "{code}"
    );
    assert!(code.contains("export const c = input();"), "{code}");
    // A namespace import isn't matched by name.
    assert!(code.contains("export const d = ng.signal(0);"), "{code}");
}

#[test]
fn partial_mode_and_advanced_optimizations_keep_it() {
    let source = r"
import { signal } from '@angular/core';
export const top = signal(1);
";
    let expected = format!("export const top = signal(1, {});", spread("top"));
    let partial = TransformOptions {
        compilation_mode: oxc_angular_compiler::CompilationMode::Partial,
        ..TransformOptions::default()
    };
    assert!(transform_with(source, &partial).contains(&expected));
    let optimized =
        TransformOptions { advanced_optimizations: true, ..TransformOptions::default() };
    assert!(transform_with(source, &optimized).contains(&expected));
    // The JIT build has no such transform (`@angular/build`'s JIT compilation
    // never adds it).
    let jit = TransformOptions { jit: true, ..TransformOptions::default() };
    assert!(transform_with(source, &jit).contains("export const top = signal(1);"));
}

#[test]
fn hoisted_binding_keeps_its_debug_name() {
    let code = transform(
        r"
import { Component, InjectionToken, signal } from '@angular/core';
export const TOK = new InjectionToken<unknown>('t');
@Component({ selector: 'a-h', template: '', providers: [{ provide: TOK, useValue: state }] })
export class H {}
const state = signal(0);
",
    );
    let expected = format!("const state = signal(0, {});", spread("state"));
    assert_eq!(code.matches(&expected).count(), 1, "{code}");
    assert!(code.find(&expected).unwrap() < code.find("class H").unwrap(), "{code}");
}

/// The output is valid JavaScript (the edits don't land in removed text).
fn assert_parses(code: &str) {
    let allocator = Allocator::default();
    let ret = oxc_parser::Parser::new(&allocator, code, oxc_span::SourceType::ts()).parse();
    assert!(ret.diagnostics.is_empty(), "{:?} in:\n{code}", ret.diagnostics);
}

#[test]
fn decorator_arguments_are_named_in_every_copy() {
    // ngtsc's definitions and `ɵsetClassMetadata` reuse the decorator's
    // argument nodes, and the transform names the calls in both.
    let source = r"
import { Component, Directive, Input, NgModule, signal } from '@angular/core';
@Component({ selector: 'x', template: '', providers: [{ provide: 'k', useFactory: () => { const fromProv = signal(0); return fromProv; } }, { provide: 'c', useValue: new (class { f = signal(0) })() }] })
export class X {}
@NgModule({ providers: [{ provide: 'm', useFactory: () => { const fromMod = signal(0); return fromMod; } }] })
export class M {}
@Directive({ selector: '[d]' })
export class D { @Input({ transform: (v: any) => { const fromTr = signal(v); return fromTr; } }) a: any; }
";
    let partial = TransformOptions {
        compilation_mode: oxc_angular_compiler::CompilationMode::Partial,
        ..TransformOptions::default()
    };
    for options in [TransformOptions::default(), partial] {
        let code = transform_with(source, &options);
        assert_parses(&code);
        for expected in [
            format!("const fromProv = signal(0, {});", spread("fromProv")),
            format!("f = signal(0, {})", spread("f")),
            format!("const fromMod = signal(0, {});", spread("fromMod")),
            // Printed again without its types.
            "debugName: \"fromTr\"".to_string(),
        ] {
            assert_eq!(code.matches(&expected).count(), 2, "`{expected}` in:\n{code}");
        }
    }

    // `ɵprov` calls the `useFactory` function.
    let code = transform(
        r"
import { Injectable, computed } from '@angular/core';
@Injectable({ providedIn: 'root', useFactory: () => { const fromInj = computed(() => 1); return fromInj; } })
export class Y {}
",
    );
    let expected = format!("const fromInj = computed(() => 1, {});", spread("fromInj"));
    assert_eq!(code.matches(&expected).count(), 2, "{code}");
}

#[test]
fn jit_forced_class() {
    let code = transform(
        r"
import { Component, ElementRef, Input, input, model, viewChild, signal } from '@angular/core';
@Component({ jit: true, selector: 'x', template: '', providers: [{ provide: 'k', useFactory: () => { const fromJit = signal(0); return fromJit; } }] })
export class X {
  name = input('a');
  req = input.required<string>();
  @Input() dec = input(1);
  m = model(0);
  q = viewChild('q', { read: ElementRef });
  s = signal(0);
}
@Component({ jit: true, selector: 'y', template: '' })
export default class {
  anon = input('a');
}
",
    );
    assert_parses(&code);
    // The decorator moves into `__decorate` with its debugName.
    let from_jit = format!("const fromJit = signal(0, {});", spread("fromJit"));
    assert_eq!(code.matches(&from_jit).count(), 1, "{code}");
    assert!(code.find(&from_jit).unwrap() > code.find("X = __decorate(").unwrap(), "{code}");
    // The JIT `signalInputsTransform` rebuilds `input()` fields under a
    // synthesized name first, so they keep no debugName.
    assert!(code.contains("name = input('a');"), "{code}");
    assert!(code.contains("req = input.required<string>();"), "{code}");
    for expected in [
        // A field with its own `@Input`, and a field of a nameless class,
        // aren't rebuilt.
        format!("dec = input(1, {})", spread("dec")),
        format!("anon = input('a', {})", spread("anon")),
        format!("m = model(0, {})", spread("m")),
        format!("q = viewChild('q', {{ {}, read: ElementRef }})", object_spread("q")),
        format!("s = signal(0, {})", spread("s")),
    ] {
        assert!(code.contains(&expected), "expected `{expected}` in:\n{code}");
    }
    // The query decorator spreads the options as written.
    assert!(code.contains("{ ...{ read: ElementRef }, isSignal: true }"), "{code}");
}

#[test]
fn service_factory_is_named() {
    let code = transform(
        r"
import { Service, signal } from '@angular/core';
@Service({ factory: () => { const fromSvc = signal(0); return new S(); } })
export class S {}
",
    );
    let expected = format!("const fromSvc = signal(0, {});", spread("fromSvc"));
    assert_eq!(code.matches(&expected).count(), 2, "{code}");
}

#[test]
fn call_and_target_shapes() {
    let code = transform(
        r"
import { signal } from '@angular/core';
export const a = signal?.(0);
export class K {
  b: any;
  c: any;
  constructor() {
    (this.b) = signal(1);
    (this).c = signal(2);
  }
}
const k = 'k';
const k2 = 'k2';
export class L { [/* c */ k] = signal(0); [ k2 // x
] = signal(1); }
",
    );
    // An optional call is a call expression upstream too.
    assert!(code.contains(&format!("export const a = signal?.(0, {});", spread("a"))), "{code}");
    // A parenthesized left side isn't a property access.
    assert!(code.contains("(this.b) = signal(1);"), "{code}");
    assert!(code.contains(&format!("(this).c = signal(2, {});", spread("c"))), "{code}");
    // `node.name.getText()` keeps the comments inside the brackets.
    assert!(code.contains(&format!("signal(0, {})", spread("[/* c */ k]"))), "{code}");
    assert!(code.contains(&format!("signal(1, {})", spread("[ k2 // x\\n]"))), "{code}");
}

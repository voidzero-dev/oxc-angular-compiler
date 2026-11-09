import { mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import type { Plugin } from 'vite'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { angular } from '../vite-plugin/index.js'
import {
  dtsSourceCandidates,
  injectDtsDeclarations,
  injectDtsDeclarationsWithIdentityMap,
  injectDtsDeclarationsWithMap,
  matchDtsSources,
} from '../vite-plugin/utils/dts.js'

const COMPONENT_SOURCE = `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-lib-button',
    template: '<button><ng-content></ng-content></button>',
    standalone: true,
  })
  export class LibButtonComponent {}
`

describe('injectDtsDeclarations', () => {
  it('splices members into the matching class and adds the i0 import', () => {
    const source = `export declare class LibButtonComponent {\n}\n`
    const out = injectDtsDeclarations(source, [
      {
        className: 'LibButtonComponent',
        members:
          'static ɵfac: i0.ɵɵFactoryDeclaration<LibButtonComponent, never>;\n' +
          'static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent, "app-lib-button", never, {}, {}, never, ["*"], true, never>;',
      },
    ])

    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<LibButtonComponent, never>;')
    expect(out).toContain(
      'static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent, "app-lib-button", never, {}, {}, never, ["*"], true, never>;',
    )
    // Members land inside the class body, before its closing brace.
    const facIdx = out.indexOf('ɵfac')
    const braceIdx = out.indexOf('class LibButtonComponent')
    const closeIdx = out.lastIndexOf('}')
    expect(braceIdx).toBeLessThan(facIdx)
    expect(facIdx).toBeLessThan(closeIdx)
  })

  it('is idempotent — re-running does not duplicate members', () => {
    const source = `export declare class Foo {\n}\n`
    const decls = [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ]
    const once = injectDtsDeclarations(source, decls)
    const twice = injectDtsDeclarations(once, decls)
    expect(twice).toBe(once)
    expect(once.match(/ɵfac/g)).toHaveLength(1)
  })

  it('reuses an existing i0 import instead of adding a second one', () => {
    const source = 'import * as i0 from "@angular/core";\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out.match(/@angular\/core/g)).toHaveLength(1)
  })

  it('appends members at the end of a class that already has members', () => {
    // Upstream adds the Ivy members after the existing ones
    // ([...members, ...newMembers]).
    const source = 'export declare class Foo {\n  constructor(x: number);\n  field: string;\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    const ctorIdx = out.indexOf('constructor')
    const facIdx = out.indexOf('ɵfac')
    const closeIdx = out.lastIndexOf('}')
    expect(ctorIdx).toBeLessThan(facIdx)
    expect(facIdx).toBeLessThan(closeIdx)
  })

  it('keeps the i0 import after leading triple-slash references', () => {
    const source = '/// <reference types="node" />\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out.indexOf('/// <reference')).toBeLessThan(out.indexOf('import * as i0'))
  })

  it('leaves files without a matching class untouched', () => {
    const source = `export declare class Other {\n}\n`
    const out = injectDtsDeclarations(source, [
      { className: 'Missing', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Missing, never>;' },
    ])
    expect(out).toBe(source)
  })

  it('dedupes the alias against identifiers in the emitted .d.ts', () => {
    // The bundle still binds `i0`, so members are normalized to the free
    // `i0_1` alias and the injected import matches (#509).
    const source = 'export declare const i0: 1;\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0_1.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out).toContain('import * as i0_1 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0_1.ɵɵFactoryDeclaration<Foo, never>;')
    expect(out).not.toContain('import * as i0 from')
  })

  it('normalizes mixed per-file aliases to one bundle namespace', () => {
    // Members compiled from different source files can carry different
    // aliases — `i0` where it was free, `i0_1` where a binding collided.
    // A bundled .d.ts needs one import, so every member is rewritten to the
    // namespace chosen for this file.
    const source = 'export declare class A {\n}\nexport declare class B {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'A', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<A, never>;' },
      { className: 'B', members: 'static ɵfac: i0_1.ɵɵFactoryDeclaration<B, never>;' },
    ])
    expect(out.match(/@angular\/core/g)).toHaveLength(1)
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<A, never>;')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<B, never>;')
    expect(out).not.toContain('i0_1')
  })

  it('never reuses an existing differently-aliased @angular/core import', () => {
    // ngtsc's declaration ImportManager mints its own `i0` even when the
    // file already has `import * as ng from "@angular/core"`.
    const source = 'import * as ng from "@angular/core";\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out.match(/@angular\/core/g)).toHaveLength(2)
    expect(out).toContain('import * as ng from "@angular/core";')
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;')
  })

  it('mints i0 even when members arrive with a hand-written ng head', () => {
    // The compiler never emits `ng` heads, but a declaration that has them
    // (older binding, hand-written input) is still normalized to the
    // canonical `i0` — upstream's ImportManager would never reuse `ng`
    // either, it dedupes `i0` against the file's identifiers.
    const source = 'import * as ng from "@angular/core";\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Foo',
        members: 'static ɵfac: ng.ɵɵFactoryDeclaration<Foo, never>;',
        namespaceImports: { ng: '@angular/core' },
      },
    ])
    expect(out).toContain('import * as ng from "@angular/core";')
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;')
  })

  it('emits a namespace import for non-core member aliases', () => {
    // `typeof i1.SomeDirective` (host directives, ctor deps) resolves through
    // `namespaceImports` — `i1` → `./dir` — and the matching import is added.
    const source = 'export declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Foo',
        members:
          'static ɵcmp: i0.ɵɵDirectiveDeclaration<Foo, never, never, {}, {}, never, never, true, [{ directive: typeof i1.SomeDirective }]>;',
        namespaceImports: { i1: './dir' },
      },
    ])
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('import * as i1 from "./dir";')
    expect(out).toContain('typeof i1.SomeDirective')
  })

  it('uniquifies non-core aliases that collide with file identifiers', () => {
    const source = 'export declare const i1: number;\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Foo',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<Foo, [typeof i1.M], never, never>;',
        namespaceImports: { i1: './dep' },
      },
    ])
    expect(out).toContain('import * as i1_1 from "./dep";')
    expect(out).toContain('typeof i1_1.M')
    expect(out).toContain('export declare const i1: number;')
  })

  it('canonicalizes the same alias to different modules per declaration', () => {
    // Members merged from two source files: `i1` meant `./dep` in one and
    // `./other` in the other — each module gets its own canonical alias.
    const source = 'export declare class A {\n}\nexport declare class B {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'A',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<A, [typeof i1.D], never, never>;',
        namespaceImports: { i1: './dep' },
      },
      {
        className: 'B',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<B, [typeof i1.O], never, never>;',
        namespaceImports: { i1: './other' },
      },
    ])
    expect(out).toContain('import * as i1 from "./dep";')
    expect(out).toContain('import * as i1_1 from "./other";')
    expect(out).toContain('typeof i1.D')
    expect(out).toContain('typeof i1_1.O')
  })

  it('keeps a trailing comment attached to the last import', () => {
    const source = 'import type { A } from "./a"; // keep me\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    const lines = out.split('\n')
    expect(lines[0]).toBe('import type { A } from "./a"; // keep me')
    expect(lines[1]).toBe('import * as i0 from "@angular/core";')
  })

  it('keeps a multiline trailing comment attached to the last import', () => {
    // The newline inside the block comment must not be chosen as the
    // insertion point — the import would land inside the comment.
    const source = 'import type { A } from "./a"; /* keep\nme */\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    const lines = out.split('\n')
    expect(lines[0]).toBe('import type { A } from "./a"; /* keep')
    expect(lines[1]).toBe('me */')
    expect(lines[2]).toBe('import * as i0 from "@angular/core";')
  })

  it('is idempotent when the incoming alias differs from the first pass', () => {
    // Members compiled against `i0_1` normalize to `i0` on the first pass;
    // the second pass sees `i0` occupied and would mint `i0_1` — the member
    // check is structural (name already in the class), so nothing is added.
    const source = 'export declare class Foo {\n}\n'
    const decls = [
      { className: 'Foo', members: 'static ɵfac: i0_1.ɵɵFactoryDeclaration<Foo, never>;' },
    ]
    const once = injectDtsDeclarations(source, decls)
    const twice = injectDtsDeclarations(once, decls)
    expect(twice).toBe(once)
    expect(once.match(/ɵfac/g)).toHaveLength(1)
    expect(once.match(/@angular\/core/g)).toHaveLength(1)
  })

  it('injects only the members the class is missing', () => {
    // A class that already has `ɵfac` (e.g. hand-authored or from an earlier
    // pass) still gets the missing `ɵcmp`, without duplicating `ɵfac`.
    const source =
      'import * as i0 from "@angular/core";\nexport declare class Foo {\n  static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Foo',
        members:
          'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;\n' +
          'static ɵcmp: i0.ɵɵComponentDeclaration<Foo, "x", never, {}, {}, never, never, true, never>;',
      },
    ])
    expect(out.match(/ɵfac/g)).toHaveLength(1)
    expect(out.match(/ɵcmp/g)).toHaveLength(1)
    expect(out.match(/@angular\/core/g)).toHaveLength(1)
  })

  it('emits the core alias for non-ɵ members via namespaceImports', () => {
    // `ngAcceptInputType_x` types like `i0.Signal<number>` aren't `ɵ` heads;
    // they resolve through `namespaceImports`.
    const source = 'export declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Foo',
        members: 'static ngAcceptInputType_x: i0.Signal<number>;',
        namespaceImports: { i0: '@angular/core' },
      },
    ])
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ngAcceptInputType_x: i0.Signal<number>;')
  })

  it('canonicalizes identical relative specifiers under different source dirs as distinct modules', () => {
    // `src/a/foo.ts` and `src/b/foo.ts` both import `"./dep"` — different
    // modules, so they get different aliases even though the raw specifier
    // strings are identical.
    const source = 'export declare class A {\n}\nexport declare class B {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'A',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<A, [typeof i1.D], never, never>;',
        namespaceImports: { i0: '@angular/core', i1: './dep' },
        sourceFile: 'src/a/foo.ts',
      },
      {
        className: 'B',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<B, [typeof i1.O], never, never>;',
        namespaceImports: { i0: '@angular/core', i1: './dep' },
        sourceFile: 'src/b/foo.ts',
      },
    ])
    expect(out).toContain('import * as i1 from "./dep";')
    expect(out).toContain('import * as i1_1 from "./dep";')
    expect(out).toContain('typeof i1.D')
    expect(out).toContain('typeof i1_1.O')
  })

  it('emits relative specifiers verbatim for structure-preserving emit', () => {
    // ngtsc and structure-preserving declaration emit place the emitted
    // `.d.ts` in the source's relative layout (`src/a/foo.ts` →
    // `dist/a/foo.d.ts`), so `"./dep"` verbatim reaches `dist/a/dep.d.ts`.
    // Rebasing to the source tree would point outside the published
    // package; under fully-bundled declaration output two same-named
    // relative modules can never both resolve regardless.
    const source = 'export declare class A {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'A',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<A, [typeof i1.D], never, never>;',
        namespaceImports: { i0: '@angular/core', i1: './dep' },
        sourceFile: '/repo/src/a/foo.ts',
      },
    ])
    expect(out).toContain('import * as i1 from "./dep";')
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('typeof i1.D')
  })

  it('reuses an existing import bound to the same verbatim specifier', () => {
    // An existing `import * as i1 from "./dep"` already binds the specifier
    // this member needs (verbatim emit), so no second import is added.
    const source = 'import * as i1 from "./dep";\nexport declare class A {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'A',
        members: 'static ɵmod: i0.ɵɵNgModuleDeclaration<A, [typeof i1.D], never, never>;',
        namespaceImports: { i0: '@angular/core', i1: './dep' },
        sourceFile: '/repo/src/a/foo.ts',
      },
    ])
    expect(out.match(/from "\.\/dep"/g)).toHaveLength(1)
    expect(out).toContain('typeof i1.D')
  })

  it('still injects a static member when an instance member has the same name', () => {
    // TypeScript permits `ɵfac` and `static ɵfac` side by side; the
    // idempotency check must only count statics, otherwise a partial build
    // loses the generated factory declaration.
    const source = 'export declare class Foo {\n  ɵfac: string;\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;')
    expect(out.match(/ɵfac/g)).toHaveLength(2)
  })

  it('keeps the generated import outside a class declared on the import line', () => {
    // `import …; export declare class Foo {` on one line must not let the
    // import insertion scan past `{` — that would splice the import into
    // the class body.
    const source = 'import type { A } from "./a"; export declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    const importIdx = out.indexOf('import * as i0')
    const classIdx = out.indexOf('export declare class Foo')
    const openBrace = out.indexOf('{', classIdx)
    expect(importIdx).toBeGreaterThan(-1)
    expect(importIdx).toBeLessThan(classIdx)
    expect(importIdx).toBeLessThan(openBrace)
  })

  it('injects into a class in a dotted ambient namespace', () => {
    // `declare namespace Outer.Inner {}` parses as nested module
    // declarations — the search must descend through them.
    const source = 'declare namespace Outer.Inner {\n  export class Foo {\n  }\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out).toContain('import * as i0 from "@angular/core";')
    const facIdx = out.indexOf('ɵfac')
    const classIdx = out.indexOf('class Foo')
    const closeIdx = out.lastIndexOf('}')
    expect(classIdx).toBeLessThan(facIdx)
    expect(facIdx).toBeLessThan(closeIdx)
  })

  it('keeps a line comment on the last import with it', () => {
    // `import …; // note` at EOF has no newline after the comment — the
    // new import must not land inside the comment.
    const source = 'import type { A } from "./a"; // note\nexport declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out).toContain('// note\nimport * as i0 from "@angular/core";')
  })

  it('infers unmapped i0 aliases as @angular/core when namespaceImports is absent', () => {
    // Legacy callers pass no `namespaceImports`; the compiler's registry
    // reserves `i0`/`i0_N` for `@angular/core`, so an unmapped `i0.Signal`
    // head must still emit the core import — previously the member was
    // injected with `i0` unresolvable.
    const source = 'export declare class Dir {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Dir',
        members: 'static ngAcceptInputType_x: i0.Signal<number>;',
      },
    ])
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('i0.Signal<number>')
  })

  it('does not infer unmapped i1+ aliases without namespaceImports', () => {
    // `i1` isn't reserved for core — it could be a relative dep. Without a
    // map the head is left alone rather than guessing a module.
    const source = 'export declare class Dir {\n}\n'
    const out = injectDtsDeclarations(source, [
      {
        className: 'Dir',
        members: 'static ngAcceptInputType_x: i1.Signal<number>;',
      },
    ])
    expect(out).not.toContain('@angular/core')
    expect(out).toContain('i1.Signal<number>')
  })

  it('handles declarations with empty or comment-only members', () => {
    // The wrapper always yields a class, but nothing should be spliced —
    // no crash, no members, no imports.
    const source = 'export declare class Foo {\n}\n'
    const decls = [
      { className: 'Foo', members: '' },
      { className: 'Foo', members: '   \n  ' },
      { className: 'Foo', members: '/* no members */ // only comments' },
    ]
    expect(injectDtsDeclarations(source, decls)).toBe(source)
  })

  it('skips declarations whose member text does not parse', () => {
    const source = 'export declare class Foo {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
      { className: 'Foo', members: 'this is not valid typescript {{' },
    ])
    // The well-formed declaration still lands; the malformed one is skipped
    // rather than injected with unnormalized aliases.
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;')
    expect(out).not.toContain('this is not valid')
  })

  it('injects into a class whose type-param constraint contains a brace', () => {
    // The AST locates the class body after `<T extends { a: 1 }>`; the old
    // regex stopped at the constraint's `{`.
    const source = 'export declare class Foo<T extends { a: 1 }> {\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out).toContain('import * as i0 from "@angular/core";')
    const facIdx = out.indexOf('ɵfac')
    const classIdx = out.indexOf('class Foo')
    const closeIdx = out.lastIndexOf('}')
    expect(classIdx).toBeLessThan(facIdx)
    expect(facIdx).toBeLessThan(closeIdx)
  })

  it('injects into a class nested in a declare module block', () => {
    const source = 'declare module "pkg" {\n  export class Foo {\n  }\n}\n'
    const out = injectDtsDeclarations(source, [
      { className: 'Foo', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<Foo, never>;' },
    ])
    expect(out).toContain('import * as i0 from "@angular/core";')
    const facIdx = out.indexOf('ɵfac')
    const classIdx = out.indexOf('class Foo')
    const closeIdx = out.lastIndexOf('}')
    expect(classIdx).toBeLessThan(facIdx)
    expect(facIdx).toBeLessThan(closeIdx)
  })
})

describe('injectDtsDeclarationsWithMap (#693)', () => {
  // From a real rolldown + rolldown-plugin-dts build.
  const source =
    'export declare class HelloComponent {\n  name: string;\n}\n' +
    'export declare class OtherComponent {\n  value: number;\n  greet(): string;\n}\n' +
    'export declare class EmptyOne {}\n//# sourceMappingURL=index.d.mts.map'
  const declarations = ['HelloComponent', 'OtherComponent', 'EmptyOne'].map((className) => ({
    className,
    members: `static ɵfac: i0.ɵɵFactoryDeclaration<${className}, never>;`,
  }))
  const map = {
    version: 3,
    file: 'index.d.mts',
    names: [],
    sourceRoot: '',
    sources: ['../src/index.ts'],
    x_google_ignoreList: [],
    mappings: 'A,qBAAa;EACX;;qBAGW;EACX;EACA;;qBAEW',
  }

  it('shifts the mappings and keeps the other map fields', () => {
    const out = injectDtsDeclarationsWithMap(source, declarations, JSON.stringify(map))
    expect(out.code).toBe(injectDtsDeclarations(source, declarations))
    expect(JSON.parse(out.map!)).toEqual({
      ...map,
      mappings: ';A,qBAAa;EACX;;;qBAGW;EACX;EACA;;;qBAEW',
    })
  })

  it('gives no map for bad map JSON or bad VLQ, but still injects', () => {
    for (const bad of ['{', '{"mappings":1}', JSON.stringify({ ...map, mappings: '!!' })]) {
      const out = injectDtsDeclarationsWithMap(source, declarations, bad)
      expect(out.code).toContain('static ɵfac')
      expect(out.map).toBeUndefined()
    }
    expect(injectDtsDeclarationsWithMap(source, declarations).map).toBeUndefined()
  })

  it('gives the map back as is when nothing is injected', () => {
    const json = JSON.stringify(map)
    const out = injectDtsDeclarationsWithMap(source, [], json)
    expect(out).toEqual({ code: source, map: json })
  })

  it('maps the new code back to the source for a transform hook', () => {
    const src = 'export declare class X {}\n'
    const out = injectDtsDeclarationsWithIdentityMap(
      src,
      [{ className: 'X', members: 'static ɵfac: i0.ɵɵFactoryDeclaration<X, never>;' }],
      'x.d.ts',
    )
    expect(out.code).toBe(
      'import * as i0 from "@angular/core";\nexport declare class X {\n' +
        '    static ɵfac: i0.ɵɵFactoryDeclaration<X, never>;\n}\n',
    )
    // Import and member lines unmapped; `export declare class X {` and `}`
    // keep their original places.
    expect(JSON.parse(out.map!)).toEqual({
      version: 3,
      sources: ['x.d.ts'],
      names: [],
      mappings: ';AAAA,OAAO,QAAQ,MAAM,EAAE;;AAAC;',
    })
    expect(injectDtsDeclarationsWithIdentityMap(src, []).map).toBeUndefined()
  })
})

describe('angular() dts plugin (#104)', () => {
  function getPlugins(compilationMode: 'full' | 'partial', options: Record<string, unknown> = {}) {
    const plugins = angular({ compilationMode, ...options })
    const transform = plugins.find((p) => p.name === '@oxc-angular/vite')
    const dts = plugins.find((p) => p.name === '@oxc-angular/vite-dts')
    if (!transform || !dts) throw new Error('missing plugins')
    return { transform, dts }
  }

  function hookOf(plugin: Plugin, name: string): (...args: any[]) => any {
    const hook = (plugin as any)[name]
    if (!hook) throw new Error(`expected ${name}`)
    return typeof hook === 'function' ? hook : hook.handler
  }

  const compileContext = {
    error(message: string) {
      throw new Error(message)
    },
    warn() {},
  }

  async function runTransform(
    transform: Plugin,
    code = COMPONENT_SOURCE,
    id = 'lib-button.component.ts',
  ) {
    return hookOf(transform, 'transform').call(compileContext as any, code, id)
  }

  function cardSource(selector: string) {
    return `
      import { Component } from '@angular/core';
      @Component({ selector: '${selector}', template: '' })
      export class Card {}
    `
  }

  // A transform context for the dts hook. `modules` are the ids in the graph;
  // `load` compiles the ones in `lazy` on demand.
  function dtsContext(modules: string[], lazy: Record<string, () => Promise<unknown>> = {}) {
    const loaded: string[] = []
    return {
      loaded,
      getModuleInfo: (id: string) => (modules.includes(id) ? { id } : null),
      async load({ id }: { id: string }) {
        loaded.push(id)
        await lazy[id]?.()
        return { id }
      },
    }
  }

  async function runDtsTransform(dts: Plugin, ctx: object, code: string, id: string) {
    return hookOf(dts, 'transform').call(ctx as any, code, id)
  }

  async function runGenerateBundle(dts: Plugin, bundle: unknown, moduleIds?: string[]) {
    const ctx = moduleIds ? { getModuleIds: () => moduleIds[Symbol.iterator]() } : {}
    await hookOf(dts, 'generateBundle').call(ctx as any, {} as any, bundle as any, false)
  }

  function asset(fileName: string, source: string | Uint8Array) {
    return { type: 'asset' as const, fileName, source }
  }

  describe('per-module transform (bundled declarations)', () => {
    it('adds each .d.ts module only its own source members', async () => {
      const { transform, dts } = getPlugins('partial')
      const b = '/p/src/b/card.component.ts'
      const c = '/p/src/c/card.component.ts'
      await runTransform(transform, cardSource('b-card'), b)
      await runTransform(transform, cardSource('c-card'), c)
      const ctx = dtsContext([b, c])

      const dtsCode = 'export declare class Card {\n}\n'
      const outB = await runDtsTransform(dts, ctx, dtsCode, '/p/src/b/card.component.d.ts')
      const outC = await runDtsTransform(dts, ctx, dtsCode, '/p/src/c/card.component.d.ts')
      expect(outB.code).toContain('import * as i0 from "@angular/core";')
      expect(outB.code).toContain('"b-card"')
      expect(outB.code).not.toContain('"c-card"')
      expect(outC.code).toContain('"c-card"')
      expect(outC.code).not.toContain('"b-card"')
      // A map back to the module's code, so the bundler chains the dts map.
      expect(JSON.parse(outB.map)).toMatchObject({
        version: 3,
        sources: ['/p/src/b/card.component.d.ts'],
      })
      expect(ctx.loaded).toEqual([])
    })

    it('leaves a .d.ts whose source has no Angular class alone', async () => {
      const { transform, dts } = getPlugins('partial')
      await runTransform(transform, COMPONENT_SOURCE, '/p/src/angular.ts')
      await runTransform(transform, 'export class Button {}', '/p/src/utils.ts')
      const ctx = dtsContext(['/p/src/angular.ts', '/p/src/utils.ts'])

      const code = 'export declare class LibButtonComponent {\n}\n'
      expect(await runDtsTransform(dts, ctx, code, '/p/src/utils.d.ts')).toBeUndefined()
    })

    it('skips a .d.ts with no source module (hand-written)', async () => {
      const { transform, dts } = getPlugins('partial')
      await runTransform(transform, COMPONENT_SOURCE, '/p/src/lib-button.component.ts')
      const ctx = dtsContext([])
      const code = 'export declare class LibButtonComponent {\n}\n'
      expect(
        await runDtsTransform(dts, ctx, code, '/p/src/lib-button.component.d.ts'),
      ).toBeUndefined()
      expect(ctx.loaded).toEqual([])
    })

    it('waits for the source transform when the .d.ts comes first', async () => {
      const { transform, dts } = getPlugins('partial')
      const source = '/p/src/lib-button.component.ts'
      const ctx = dtsContext([source], {
        [source]: () => runTransform(transform, COMPONENT_SOURCE, source),
      })
      const out = await runDtsTransform(
        dts,
        ctx,
        'export declare class LibButtonComponent {\n}\n',
        '/p/src/lib-button.component.d.ts',
      )
      expect(ctx.loaded).toEqual([source])
      expect(out.code).toContain('static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent')
    })

    it('finds a .tsx source', async () => {
      const { transform, dts } = getPlugins('partial')
      const source = '/p/src/lib-button.component.tsx'
      await runTransform(transform, COMPONENT_SOURCE, source)
      const out = await runDtsTransform(
        dts,
        dtsContext([source]),
        'export declare class LibButtonComponent {\n}\n',
        '/p/src/lib-button.component.d.ts',
      )
      expect(out.code).toContain('static ɵfac')
    })

    it('does nothing in full (app) mode', async () => {
      const { transform, dts } = getPlugins('full')
      const source = '/p/src/lib-button.component.ts'
      await runTransform(transform, COMPONENT_SOURCE, source)
      const out = await runDtsTransform(
        dts,
        dtsContext([source]),
        'export declare class LibButtonComponent {\n}\n',
        '/p/src/lib-button.component.d.ts',
      )
      expect(out).toBeUndefined()
    })

    it('runs before other transforms, and not on node_modules or virtual ids', () => {
      const { dts } = getPlugins('partial')
      const hook = (dts as any).transform
      expect(hook.order).toBe('pre')
      const { include, exclude } = hook.filter.id
      expect(
        include.test('/p/a.d.ts') && include.test('/p/a.d.mts') && include.test('/p/a.d.cts'),
      ).toBe(true)
      expect(include.test('/p/a.ts')).toBe(false)
      expect(exclude.test('/p/node_modules/x/a.d.ts')).toBe(true)
      expect(exclude.test('\0virtual.d.ts')).toBe(true)
    })

    it('maps .d.ts ids back to their source ids', () => {
      expect(dtsSourceCandidates('/p/a.d.ts')).toEqual(['/p/a.ts', '/p/a.tsx'])
      expect(dtsSourceCandidates('/p/a.d.mts')).toEqual(['/p/a.mts'])
      expect(dtsSourceCandidates('/p/a.d.cts')).toEqual(['/p/a.cts'])
      expect(dtsSourceCandidates('/p/a.ts')).toEqual([])
    })
  })

  describe('generateBundle (assets)', () => {
    it('leaves chunks alone, even when a class name matches', async () => {
      const { transform, dts } = getPlugins('partial')
      await runTransform(transform, COMPONENT_SOURCE, '/p/src/index.ts')
      const code = 'export declare class LibButtonComponent {\n}\n'
      const bundle = {
        'index.d.ts': { type: 'chunk' as const, fileName: 'index.d.ts', code },
      }
      await runGenerateBundle(dts, bundle, ['/p/src/index.ts'])
      expect(bundle['index.d.ts'].code).toBe(code)
    })

    it('adds members to the asset of the same source path', async () => {
      const { transform, dts } = getPlugins('partial')
      const b = '/p/src/b/card.component.ts'
      const c = '/p/src/c/card.component.ts'
      await runTransform(transform, cardSource('b-card'), b)
      await runTransform(transform, cardSource('c-card'), c)
      const code = 'export declare class Card {\n}\n'
      const bundle = {
        'types/b/card.component.d.ts': asset('types/b/card.component.d.ts', code),
        'types/c/card.component.d.ts': asset('types/c/card.component.d.ts', code),
        'types/index.d.ts': asset('types/index.d.ts', code),
      }
      await runGenerateBundle(dts, bundle, ['/p/src/index.ts', b, c])

      const outB = bundle['types/b/card.component.d.ts'].source as string
      const outC = bundle['types/c/card.component.d.ts'].source as string
      expect(outB).toContain('"b-card"')
      expect(outB).not.toContain('"c-card"')
      expect(outC).toContain('"c-card"')
      expect(outC).not.toContain('"b-card"')
      // Same class name, other source file: untouched.
      expect(bundle['types/index.d.ts'].source).toBe(code)
    })

    it('reads Uint8Array asset sources', async () => {
      const { transform, dts } = getPlugins('partial')
      await runTransform(transform, COMPONENT_SOURCE, '/p/src/lib-button.component.ts')
      const code = 'export declare class LibButtonComponent {\n}\n'
      const bundle = {
        'lib-button.component.d.ts': asset(
          'lib-button.component.d.ts',
          new TextEncoder().encode(code),
        ),
      }
      await runGenerateBundle(dts, bundle, ['/p/src/lib-button.component.ts'])
      expect(bundle['lib-button.component.d.ts'].source).toContain('static ɵcmp')
    })

    it('leaves the .d.ts of a plain module alone', async () => {
      const { transform, dts } = getPlugins('partial')
      // An Angular `Button` in lib/utils.ts, a plain one in utils.ts.
      const angularButton = COMPONENT_SOURCE.replace('LibButtonComponent', 'Button')
      await runTransform(transform, angularButton, '/p/src/lib/utils.ts')
      const code = 'export declare class Button {\n}\n'
      const bundle = { 'types/utils.d.ts': asset('types/utils.d.ts', code) }
      await runGenerateBundle(dts, bundle, ['/p/src/utils.ts', '/p/src/lib/utils.ts'])
      expect(bundle['types/utils.d.ts'].source).toBe(code)
    })

    it('skips an asset two sources could have written', async () => {
      const { transform, dts } = getPlugins('partial')
      const b = '/p/src/b/card.component.ts'
      const c = '/p/src/c/card.component.ts'
      await runTransform(transform, cardSource('b-card'), b)
      await runTransform(transform, cardSource('c-card'), c)
      const code = 'export declare class Card {\n}\n'
      const bundle = { 'card.component.d.ts': asset('card.component.d.ts', code) }
      await runGenerateBundle(dts, bundle, [b, c])
      expect(bundle['card.component.d.ts'].source).toBe(code)
    })

    describe('declaration maps', () => {
      const code = 'export declare class LibButtonComponent {\n}\n'
      const map = {
        version: 3,
        file: 'lib-button.component.d.ts',
        sourceRoot: '',
        sources: ['../src/lib-button.component.ts'],
        names: [],
        mappings: 'AAAA,qBAAa;AAAb',
      }
      const shifted = ';AAAA,qBAAa;;;AAAb'

      async function augment(bundle: Record<string, unknown>) {
        const { transform, dts } = getPlugins('partial')
        await runTransform(transform, COMPONENT_SOURCE, '/p/src/lib-button.component.ts')
        await runGenerateBundle(dts, bundle, ['/p/src/lib-button.component.ts'])
      }

      it('shifts a <file>.map asset', async () => {
        for (const mapSource of [
          JSON.stringify(map),
          new TextEncoder().encode(JSON.stringify(map)),
        ]) {
          const bundle = {
            'types/lib-button.component.d.ts': asset('types/lib-button.component.d.ts', code),
            'types/lib-button.component.d.ts.map': asset(
              'types/lib-button.component.d.ts.map',
              mapSource,
            ),
          }
          await augment(bundle)
          expect(bundle['types/lib-button.component.d.ts'].source).toContain('static ɵcmp')
          expect(
            JSON.parse(bundle['types/lib-button.component.d.ts.map'].source as string),
          ).toEqual({
            ...map,
            mappings: shifted,
          })
        }
      })

      it('follows a URL-escaped sourceMappingURL', async () => {
        const withUrl = `${code}//# sourceMappingURL=maps/lib%20button.d.ts.map`
        const bundle = {
          'types/lib-button.component.d.ts': asset('types/lib-button.component.d.ts', withUrl),
          'types/maps/lib button.d.ts.map': asset(
            'types/maps/lib button.d.ts.map',
            JSON.stringify(map),
          ),
        }
        await augment(bundle)
        expect(bundle['types/lib-button.component.d.ts'].source).toContain(
          '//# sourceMappingURL=maps/lib%20button.d.ts.map',
        )
        expect(JSON.parse(bundle['types/maps/lib button.d.ts.map'].source as string).mappings).toBe(
          shifted,
        )
      })

      it('rewrites an inline map and keeps its prefix', async () => {
        const prefix = 'data:application/json;charset=utf-8;base64,'
        const inline = (json: object) =>
          prefix + Buffer.from(JSON.stringify(json)).toString('base64')
        const bundle = {
          'lib-button.component.d.ts': asset(
            'lib-button.component.d.ts',
            `${code}//# sourceMappingURL=${inline(map)}`,
          ),
        }
        await augment(bundle)
        const out = bundle['lib-button.component.d.ts'].source as string
        expect(out).toContain('static ɵcmp')
        expect(out.endsWith(`//# sourceMappingURL=${inline({ ...map, mappings: shifted })}`)).toBe(
          true,
        )
      })

      it('leaves a bad map alone and still injects', async () => {
        const bundle = {
          'lib-button.component.d.ts': asset('lib-button.component.d.ts', code),
          'lib-button.component.d.ts.map': asset('lib-button.component.d.ts.map', '{'),
        }
        await augment(bundle)
        expect(bundle['lib-button.component.d.ts'].source).toContain('static ɵcmp')
        expect(bundle['lib-button.component.d.ts.map'].source).toBe('{')
      })

      it('injects without a map', async () => {
        const bundle = { 'lib-button.component.d.ts': asset('lib-button.component.d.ts', code) }
        await augment(bundle)
        expect(bundle['lib-button.component.d.ts'].source).toContain('static ɵcmp')
      })
    })

    it('does not touch declarations in full (app) mode', async () => {
      const { transform, dts } = getPlugins('full')
      await runTransform(transform, COMPONENT_SOURCE, '/p/src/lib-button.component.ts')
      const original = 'export declare class LibButtonComponent {\n}\n'
      const bundle = { 'lib-button.component.d.ts': asset('lib-button.component.d.ts', original) }
      await runGenerateBundle(dts, bundle, ['/p/src/lib-button.component.ts'])
      expect(bundle['lib-button.component.d.ts'].source).toBe(original)
    })
  })

  describe('matchDtsSources', () => {
    const match = (fileNames: string[], ids: string[], caseSensitive?: boolean) =>
      matchDtsSources(
        fileNames.map((fileName) => ({ fileName })),
        ids,
        caseSensitive,
      )

    it('matches each file to the source at the same path from one root', () => {
      const ids = ['/p/src/index.ts', '/p/src/b/index.ts']
      expect(match(['types/index.d.ts', 'types/b/index.d.ts', 'types/c/index.d.ts'], ids)).toEqual([
        '/p/src/index.ts',
        '/p/src/b/index.ts',
        undefined,
      ])
    })

    it('leaves a file whose source is not in the graph alone (test double)', () => {
      // vite-plugin-dts writes every file in tsconfig `include`, also
      // src/testing/button.ts, which the bundle doesn't import.
      const ids = ['/p/src/index.ts', '/p/src/button.ts']
      expect(match(['index.d.ts', 'button.d.ts', 'testing/button.d.ts'], ids)).toEqual([
        '/p/src/index.ts',
        '/p/src/button.ts',
        undefined,
      ])
      // Same with the graph's common dir below the generator's root.
      const lib = ['/p/src/lib/index.ts', '/p/src/lib/card.component.ts']
      expect(
        match(['lib/index.d.ts', 'lib/card.component.d.ts', 'testing/card.component.d.ts'], lib),
      ).toEqual([...lib, undefined])
    })

    it('finds the root below the graph’s common dir (vite-plugin-dts entryRoot)', () => {
      // entryRoot 'src', and src/button.ts imports ../shared/util.ts.
      const ids = ['/p/src/index.ts', '/p/src/button.ts', '/p/shared/util.ts']
      expect(match(['index.d.ts', 'button.d.ts'], ids)).toEqual(ids.slice(0, 2))
      // Without entryRoot.
      expect(match(['src/index.d.ts', 'src/button.d.ts', 'shared/util.d.ts'], ids)).toEqual(ids)
    })

    it('takes the source a declaration map names, and only that', () => {
      const ids = ['/p/src/button.ts', '/p/src/index.ts']
      const files = [
        { fileName: 'index.d.ts' },
        // The map wins over the layout.
        { fileName: 'testing/button.d.ts', mapSource: '/p/src/button.ts' },
        // A map naming a file outside the graph: left alone.
        { fileName: 'button.d.ts', mapSource: '/p/src/testing/button.ts' },
      ]
      expect(matchDtsSources(files, ids)).toEqual([
        '/p/src/index.ts',
        '/p/src/button.ts',
        undefined,
      ])
    })

    it('breaks a tie by the root that holds more sources, as tsc’s common dir', () => {
      expect(match(['types/index.d.ts'], ['/p/src/index.ts', '/p/src/b/index.ts'])).toEqual([
        '/p/src/index.ts',
      ])
      // A plain src/utils.ts claims utils.d.ts from an Angular src/lib/utils.ts.
      expect(match(['types/utils.d.ts'], ['/p/src/utils.ts', '/p/src/lib/utils.ts'])).toEqual([
        '/p/src/utils.ts',
      ])
      // Two equal roots: nothing.
      expect(
        match(
          ['card.component.d.ts'],
          ['/p/src/b/card.component.ts', '/p/src/c/card.component.ts'],
        ),
      ).toEqual([undefined])
    })

    it('needs the same flavor and ignores non-sources', () => {
      const ids = [
        '/p/src/a.mts',
        '/p/src/b.ts',
        '/p/src/c.d.ts',
        '/p/node_modules/x/d.ts',
        '\0e.ts',
      ]
      for (const fileName of ['a.d.ts', 'b.d.cts', 'c.d.ts', 'd.d.ts', 'e.d.ts']) {
        expect(match(['a.d.mts', fileName], ids)).toEqual(['/p/src/a.mts', undefined])
      }
    })

    it('splits on both separators and folds case when asked', () => {
      const ids = ['/p/src/b/Card.ts', '/p/src/c/card.ts']
      expect(match(['types\\b\\Card.d.ts', 'types/c/card.d.ts'], ids)).toEqual(ids)
      expect(match(['types/b/card.d.ts', 'types/c/card.d.ts'], ids)).toEqual([undefined, ids[1]])
      expect(match(['types/b/card.d.ts', 'types/c/card.d.ts'], ids, false)).toEqual(ids)
    })

    it('handles thousands of files quickly', async () => {
      const { transform, dts } = getPlugins('partial')
      const n = 2000
      const ids = Array.from({ length: n }, (_, i) => `/p/src/d${i % 40}/file${i}.ts`)
      await runTransform(transform, cardSource('x-card'), ids[n - 1])
      const code = 'export declare class Card {\n}\n'
      const bundle = Object.fromEntries(
        ids.map((id) => {
          const fileName = `types/${id.slice('/p/src/'.length, -3)}.d.ts`
          return [fileName, asset(fileName, code)]
        }),
      )
      const start = performance.now()
      await runGenerateBundle(dts, bundle, ids)
      expect(performance.now() - start).toBeLessThan(500)
      expect(bundle[`types/d39/file${n - 1}.d.ts`].source).toContain('"x-card"')
      expect(bundle['types/d0/file0.d.ts'].source).toBe(code)
    })
  })

  describe('writeBundle (files written by the generator)', () => {
    const dtsCode = 'export declare class Card {\n}\n'
    let root: string

    beforeEach(() => {
      root = realpathSync(mkdtempSync(join(tmpdir(), 'oxc-angular-dts-')))
    })
    afterEach(() => {
      rmSync(root, { recursive: true, force: true })
    })

    function write(file: string, content: string) {
      mkdirSync(dirname(join(root, file)), { recursive: true })
      writeFileSync(join(root, file), content)
    }
    function read(file: string) {
      return readFileSync(join(root, file), 'utf8')
    }

    async function setup(options: Record<string, unknown> = {}, moreIds: string[] = []) {
      const plugins = getPlugins('partial', { workspaceRoot: root, ...options })
      const b = join(root, 'src/b/card.component.ts')
      const c = join(root, 'src/c/card.component.ts')
      await runTransform(plugins.transform, cardSource('b-card'), b)
      await runTransform(plugins.transform, cardSource('c-card'), c)
      const ids = [b, c, join(root, 'src/index.ts'), ...moreIds.map((id) => join(root, id))]
      const warnings: string[] = []
      return {
        ...plugins,
        warnings,
        writeBundle: (output: object, bundle: object = {}, outDir?: string) => {
          const ctx = {
            getModuleIds: () => ids[Symbol.iterator](),
            environment: outDir ? { config: { build: { outDir } } } : undefined,
            warn: (message: string) => warnings.push(message),
          }
          return hookOf(plugins.dts, 'writeBundle').call(ctx as any, output, bundle)
        },
      }
    }

    it('runs after other writeBundle hooks', () => {
      const { dts } = getPlugins('partial')
      expect((dts as any).writeBundle.order).toBe('post')
    })

    it('adds members to the files in the output dir by path', async () => {
      const { writeBundle, warnings } = await setup()
      write('dist/b/card.component.d.ts', dtsCode)
      write('dist/c/card.component.d.ts', dtsCode)
      write('dist/index.d.ts', dtsCode)
      write('dist/node_modules/pkg/b/card.component.d.ts', dtsCode)
      write('dist/.cache/b/card.component.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'dist') })

      expect(read('dist/b/card.component.d.ts')).toContain('"b-card"')
      expect(read('dist/b/card.component.d.ts')).not.toContain('"c-card"')
      expect(read('dist/c/card.component.d.ts')).toContain('"c-card"')
      expect(read('dist/index.d.ts')).toBe(dtsCode)
      expect(read('dist/node_modules/pkg/b/card.component.d.ts')).toBe(dtsCode)
      expect(read('dist/.cache/b/card.component.d.ts')).toBe(dtsCode)
      expect(warnings).toEqual([])
    })

    it('leaves a file whose source is not in the graph alone', async () => {
      const { writeBundle } = await setup()
      // A plain `Card` test double the library doesn't import.
      write('dist/b/card.component.d.ts', dtsCode)
      write('dist/index.d.ts', dtsCode)
      write('dist/testing/b/card.component.d.ts', dtsCode)
      write('dist/testing/card.component.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toContain('"b-card"')
      expect(read('dist/testing/b/card.component.d.ts')).toBe(dtsCode)
      expect(read('dist/testing/card.component.d.ts')).toBe(dtsCode)
    })

    it('finds files under vite-plugin-dts’s entryRoot with a source outside it', async () => {
      // entryRoot 'src', and the graph also has ../shared/util.ts.
      const { writeBundle, warnings } = await setup({}, ['shared/util.ts'])
      write('dist/b/card.component.d.ts', dtsCode)
      write('dist/c/card.component.d.ts', dtsCode)
      write('dist/index.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toContain('"b-card"')
      expect(read('dist/c/card.component.d.ts')).toContain('"c-card"')
      expect(warnings).toEqual([])
    })

    it('matches by the declaration map first', async () => {
      const { writeBundle } = await setup()
      const withMap = (source: string) =>
        JSON.stringify({ version: 3, sources: [source], names: [], mappings: 'AAAA' })
      // The map says where each file came from, whatever the path.
      write('dist/x/card.component.d.ts', dtsCode)
      write('dist/x/card.component.d.ts.map', withMap('../../src/c/card.component.ts'))
      // A map naming a file outside the graph: left alone.
      write('dist/b/card.component.d.ts', dtsCode)
      write('dist/b/card.component.d.ts.map', withMap('../../src/testing/card.component.ts'))
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/x/card.component.d.ts')).toContain('"c-card"')
      expect(read('dist/b/card.component.d.ts')).toBe(dtsCode)
    })

    it('finds one layout in each scanned dir', async () => {
      const { writeBundle, warnings } = await setup()
      // Rolldown's output dir and Vite's build.outDir apart, with other layouts.
      write('out/types/b/card.component.d.ts', dtsCode)
      write('out/types/c/card.component.d.ts', dtsCode)
      write('dist/src/b/card.component.d.ts', dtsCode)
      write('dist/src/c/card.component.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'out') }, {}, 'dist')
      expect(read('out/types/b/card.component.d.ts')).toContain('"b-card"')
      expect(read('out/types/c/card.component.d.ts')).toContain('"c-card"')
      expect(read('dist/src/b/card.component.d.ts')).toContain('"b-card"')
      expect(read('dist/src/c/card.component.d.ts')).toContain('"c-card"')
      expect(warnings).toEqual([])
    })

    it('warns once per dir when no file matches', async () => {
      const { writeBundle, warnings } = await setup()
      write('dist/card.component.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'dist') })
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/card.component.d.ts')).toBe(dtsCode)
      expect(warnings).toHaveLength(1)
      expect(warnings[0]).toContain(join(root, 'dist'))
      // No declaration files of another generator: nothing to say.
      const other = await setup()
      await other.writeBundle({ dir: join(root, 'empty') })
      expect(other.warnings).toEqual([])
    })

    it('scans Vite’s build.outDir when the output dir is below it', async () => {
      const { writeBundle } = await setup()
      write('dist/b/card.component.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'dist/esm') }, {}, 'dist')
      expect(read('dist/b/card.component.d.ts')).toContain('"b-card"')
    })

    it('updates the sibling map', async () => {
      const { writeBundle } = await setup()
      const map = {
        version: 3,
        sources: ['../../src/b/card.component.ts'],
        names: [],
        mappings: 'AAAA;AAAA',
      }
      write('dist/b/card.component.d.ts', `${dtsCode}//# sourceMappingURL=card.component.d.ts.map`)
      write('dist/b/card.component.d.ts.map', JSON.stringify(map))
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toContain('"b-card"')
      expect(JSON.parse(read('dist/b/card.component.d.ts.map'))).toEqual({
        ...map,
        mappings: ';AAAA;;;AAAA',
      })
    })

    it('never rewrites files in or above the project root', async () => {
      const { writeBundle } = await setup()
      write('b/card.component.d.ts', dtsCode)
      await writeBundle({ dir: root })
      await writeBundle({ dir: dirname(root) })
      expect(read('b/card.component.d.ts')).toBe(dtsCode)
    })

    it('is idempotent, also when two passes overlap', async () => {
      const { writeBundle } = await setup()
      write('dist/b/card.component.d.ts', dtsCode)
      await Promise.all([
        writeBundle({ dir: join(root, 'dist') }),
        writeBundle({ dir: join(root, 'dist') }),
      ])
      const once = read('dist/b/card.component.d.ts')
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toBe(once)
      expect(once.match(/import \* as i0/g)).toHaveLength(1)
      expect(once.match(/static ɵcmp/g)).toHaveLength(1)
    })

    it('skips the bundle’s own declaration chunks as written', async () => {
      const { writeBundle } = await setup()
      // A bundled chunk at a path that matches a source: already done in `transform`.
      write('dist/b/card.component.d.ts', dtsCode)
      const bundle = {
        'b/card.component.d.ts': {
          type: 'chunk',
          fileName: 'b/card.component.d.ts',
          code: dtsCode,
        },
      }
      await writeBundle({ dir: join(root, 'dist') }, bundle)
      expect(read('dist/b/card.component.d.ts')).toBe(dtsCode)
    })

    it('does nothing when tsdown generates the declarations', async () => {
      const { dts, writeBundle } = await setup()
      hookOf(dts, 'tsdownConfigResolved')({ dts: {} })
      write('dist/b/card.component.d.ts', dtsCode)
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toBe(dtsCode)
    })

    it('does nothing in a tsdown without plugin hooks (< 0.21.9)', async () => {
      const { dts, writeBundle, warnings } = await setup()
      hookOf(dts, 'options')({ plugins: [{ name: 'other' }, [[{ name: 'tsdown:shebang' }]]] })
      write('dist/b/card.component.d.ts', dtsCode)
      write('dist/index.d.cts', dtsCode)
      await writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toBe(dtsCode)
      expect(warnings).toEqual([])

      // A newer tsdown without `dts`: other generators still get the pass.
      const newer = await setup()
      hookOf(newer.dts, 'options')({ plugins: [{ name: 'tsdown:shebang' }] })
      await hookOf(newer.dts, 'tsdownConfig')({})
      hookOf(newer.dts, 'tsdownConfigResolved')({ dts: false })
      await newer.writeBundle({ dir: join(root, 'dist') })
      expect(read('dist/b/card.component.d.ts')).toContain('"b-card"')
    })

    it('does nothing in full (app) mode', async () => {
      const { transform, dts } = getPlugins('full', { workspaceRoot: root })
      await runTransform(transform, cardSource('b-card'), join(root, 'src/b/card.component.ts'))
      write('dist/b/card.component.d.ts', dtsCode)
      await hookOf(dts, 'writeBundle').call({} as any, { dir: join(root, 'dist') }, {})
      expect(read('dist/b/card.component.d.ts')).toBe(dtsCode)
    })
  })

  describe('tsdown cjs declaration pass', () => {
    type InputOptions = (
      options: { plugins?: unknown; [key: string]: unknown },
      format: string,
      context: { cjsDts: boolean },
    ) => Promise<{ plugins?: unknown; [key: string]: unknown }>

    async function configure(
      config: Record<string, unknown>,
      mode: 'full' | 'partial' = 'partial',
    ) {
      const { dts } = getPlugins(mode)
      await hookOf(dts, 'tsdownConfig')(config)
      return config.inputOptions as InputOptions
    }

    function names(plugins: unknown): string[] {
      return (plugins as unknown[]).flat(Infinity).map((p: any) => p.name)
    }

    it('adds a fresh angular() to the cjs declaration pass only', async () => {
      const inputOptions = await configure({})
      const own = { name: 'tsdown:own' }
      const js = await inputOptions({ plugins: [own] }, 'cjs', { cjsDts: false })
      expect(names(js.plugins)).toEqual(['tsdown:own'])
      const dtsPass = await inputOptions({ plugins: [own] }, 'cjs', { cjsDts: true })
      expect(names(dtsPass.plugins)).toEqual([
        '@oxc-angular/vite',
        '@oxc-angular/vite-dts',
        'tsdown:own',
      ])
    })

    it('keeps the user’s inputOptions, as a function or an object', async () => {
      const calls: unknown[] = []
      const fn = await configure({
        inputOptions(options: object, format: string, context: object) {
          calls.push([format, context])
          return { ...options, treeshake: false }
        },
      })
      const out = await fn({ plugins: [] }, 'cjs', { cjsDts: true })
      expect(out.treeshake).toBe(false)
      expect(names(out.plugins)).toContain('@oxc-angular/vite-dts')
      expect(calls).toEqual([['cjs', { cjsDts: true }]])
      // Other passes: exactly what the function gives.
      const js = await fn({ plugins: [{ name: 'tsdown:own' }], external: ['y'] }, 'esm', {
        cjsDts: false,
      })
      expect(js).toEqual({ plugins: [{ name: 'tsdown:own' }], external: ['y'], treeshake: false })
    })

    describe('object inputOptions', () => {
      let cwd: string
      beforeEach(() => {
        cwd = realpathSync(mkdtempSync(join(tmpdir(), 'oxc-angular-tsdown-')))
      })
      afterEach(() => {
        rmSync(cwd, { recursive: true, force: true })
      })

      // A tsdown whose `mergeConfig` follows tsdown 0.23: `plugins` are
      // [defaults, user], any other array of the user replaces the default.
      function installTsdown() {
        const dir = join(cwd, 'node_modules/tsdown')
        mkdirSync(join(dir, 'dist'), { recursive: true })
        writeFileSync(
          join(dir, 'package.json'),
          JSON.stringify({
            name: 'tsdown',
            type: 'module',
            exports: { './config': './dist/config.mjs' },
          }),
        )
        writeFileSync(
          join(dir, 'dist/config.mjs'),
          `globalThis.__tsdownMergeCalls = []
          export function mergeConfig(defaults, user) {
            globalThis.__tsdownMergeCalls.push([defaults, user])
            const out = { ...defaults }
            for (const [key, value] of Object.entries(user)) {
              out[key] = key === 'plugins' ? [].concat(defaults[key], value) : value
            }
            return out
          }`,
        )
      }

      it('merges it with tsdown’s own mergeConfig on every pass', async () => {
        installTsdown()
        const user = { plugins: [{ name: 'inputOptions-plugin' }], external: ['x'] }
        const inputOptions = await configure({ cwd, inputOptions: user })
        const defaults = () => ({
          plugins: [{ name: 'tsdown:own' }, [{ name: 'user-plugin' }]],
          external: ['y'],
        })

        const js = await inputOptions(defaults(), 'esm', { cjsDts: false })
        expect(names(js.plugins)).toEqual(['tsdown:own', 'user-plugin', 'inputOptions-plugin'])
        expect(js.external).toEqual(['x'])
        const calls = (globalThis as any).__tsdownMergeCalls
        expect(calls).toHaveLength(1)
        expect(calls[0][1]).toBe(user)

        const dtsPass = await inputOptions(defaults(), 'cjs', { cjsDts: true })
        expect(names(dtsPass.plugins)).toEqual([
          '@oxc-angular/vite',
          '@oxc-angular/vite-dts',
          'tsdown:own',
          'user-plugin',
          'inputOptions-plugin',
        ])
        expect(dtsPass.external).toEqual(['x'])
      })

      it('leaves it alone when tsdown can’t be loaded', async () => {
        const user = { external: ['x'] }
        const config = { cwd, inputOptions: user }
        await configure(config)
        expect(config.inputOptions).toBe(user)
      })
    })

    it('wraps once when the config is set up twice', async () => {
      const config: Record<string, unknown> = {}
      await configure(config)
      const first = config.inputOptions
      await configure(config)
      expect(config.inputOptions).toBe(first)
      const out = await (config.inputOptions as InputOptions)({ plugins: [] }, 'cjs', {
        cjsDts: true,
      })
      expect(names(out.plugins).filter((n) => n === '@oxc-angular/vite-dts')).toHaveLength(1)
    })

    it('does not wrap in full (app) mode', async () => {
      const config: Record<string, unknown> = {}
      await configure(config, 'full')
      expect(config.inputOptions).toBeUndefined()
    })

    it('the added instance keeps the source and adds members to the .d.ts', async () => {
      const inputOptions = await configure({})
      const { plugins } = await inputOptions({ plugins: [] }, 'cjs', { cjsDts: true })
      const [transform, dts] = (plugins as Plugin[][])[0]
      const source = '/p/src/lib-button.component.ts'
      // rolldown-plugin-dts then reads the source, not compiled code.
      expect(await runTransform(transform, COMPONENT_SOURCE, source)).toBeUndefined()
      const out = await runDtsTransform(
        dts,
        dtsContext([source]),
        'export declare class LibButtonComponent {\n}\n',
        '/p/src/lib-button.component.d.ts',
      )
      expect(out.code).toContain('static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent')
      // tsdown writes its declarations: no disk pass.
      expect(hookOf(dts, 'writeBundle').call({} as any, { dir: '/p/dist' }, {})).toBeUndefined()
    })
  })
})

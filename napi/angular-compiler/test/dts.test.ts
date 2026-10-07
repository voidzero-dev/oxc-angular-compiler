import type { Plugin } from 'vite'
import { describe, expect, it } from 'vitest'

import { angular } from '../vite-plugin/index.js'
import { injectDtsDeclarations } from '../vite-plugin/utils/dts.js'

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

describe('angular() dts plugin (#104)', () => {
  function getPlugins(compilationMode: 'full' | 'partial') {
    const plugins = angular({ compilationMode })
    const transform = plugins.find((p) => p.name === '@oxc-angular/vite')
    const dts = plugins.find((p) => p.name === '@oxc-angular/vite-dts')
    if (!transform || !dts) throw new Error('missing plugins')
    return { transform, dts }
  }

  async function runTransform(transform: Plugin) {
    if (!transform.transform || typeof transform.transform === 'function') {
      throw new Error('expected transform handler')
    }
    await transform.transform.handler.call(
      {
        error(message: string) {
          throw new Error(message)
        },
        warn() {},
      } as any,
      COMPONENT_SOURCE,
      'lib-button.component.ts',
    )
  }

  function makeBundle(dtsSource: string) {
    return {
      'index.d.ts': {
        type: 'asset' as const,
        fileName: 'index.d.ts',
        source: dtsSource,
      },
    }
  }

  async function runGenerateBundle(dts: Plugin, bundle: unknown) {
    const hook = dts.generateBundle
    if (!hook) throw new Error('expected generateBundle')
    const fn = typeof hook === 'function' ? hook : hook.handler
    await fn.call({} as any, {} as any, bundle as any, false)
  }

  it('augments .d.ts assets in partial mode', async () => {
    const { transform, dts } = getPlugins('partial')
    await runTransform(transform)

    const bundle = makeBundle('export declare class LibButtonComponent {\n}\n')
    await runGenerateBundle(dts, bundle)

    const out = bundle['index.d.ts'].source as string
    expect(out).toContain('import * as i0 from "@angular/core";')
    expect(out).toContain('static ɵfac: i0.ɵɵFactoryDeclaration<LibButtonComponent')
    expect(out).toContain('static ɵcmp: i0.ɵɵComponentDeclaration<LibButtonComponent')
  })

  it('does not touch declarations in full (app) mode', async () => {
    const { transform, dts } = getPlugins('full')
    await runTransform(transform)

    const original = 'export declare class LibButtonComponent {\n}\n'
    const bundle = makeBundle(original)
    await runGenerateBundle(dts, bundle)

    expect(bundle['index.d.ts'].source).toBe(original)
  })
})

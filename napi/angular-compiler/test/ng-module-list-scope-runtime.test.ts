/**
 * NgModule scopes Oxc reads across files (#703), at runtime with the real
 * `@angular/core` and `ngJitMode = false`.
 *
 * With cross-file analysis, the full-mode `ɵɵsetNgModuleScope` lists the
 * classes a list gives, like ngtsc (`forwardRef(() => ns.X)`, spreads of
 * imported lists, `ns.X.forRoot()`), instead of the list as written. Oxc
 * runs that call in production too, so the listed classes must give a
 * declared component the same directives. A list with an entry Oxc can't
 * read stays as written, next to lists that were read. When the file of a
 * listed class loads this one back, the lists stay lazy (`() => [...]`).
 */
import { execFileSync } from 'node:child_process'
import { mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath, pathToFileURL } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { transformAngularFileSync } from '../index.js'

const TMP = join(fileURLToPath(new URL('.', import.meta.url)), '.ng-module-list-scope-tmp')
let nextCase = 0

beforeAll(() => {
  mkdirSync(TMP, { recursive: true })
})

afterAll(() => {
  rmSync(TMP, { recursive: true, force: true })
})

/**
 * Writes `files` (`.ts` sources importing each other as `./name`), compiles
 * each in full mode with cross-file analysis, and writes it as `name.mjs`.
 * Returns the directory and the compiled code of each file.
 */
function build(files: Record<string, string>): { dir: string; code: Record<string, string> } {
  const dir = join(TMP, `case-${nextCase++}`)
  mkdirSync(dir, { recursive: true })
  for (const [name, source] of Object.entries(files)) {
    writeFileSync(join(dir, `${name}.ts`), source)
  }
  const code: Record<string, string> = {}
  for (const [name, source] of Object.entries(files)) {
    const result = transformAngularFileSync(source, join(dir, `${name}.ts`), {
      resolveImportedValues: true,
    })
    expect(result.errors).toEqual([])
    expect(result.code).not.toContain('ngJitMode')
    code[name] = result.code
    writeMjs(dir, name, result.code, Object.keys(files))
  }
  return { dir, code }
}

/** Writes `code` as `name.mjs`, its imports of `names` pointing at `.mjs`. */
function writeMjs(dir: string, name: string, code: string, names: string[]) {
  let out = code
  for (const other of names) {
    out = out
      .replaceAll(`'./${other}'`, `'./${other}.mjs'`)
      .replaceAll(`"./${other}"`, `"./${other}.mjs"`)
  }
  writeFileSync(join(dir, `${name}.mjs`), out)
}

/**
 * Loads `first`, then prints, sorted, the directives the template of `cmp` (an export
 * of `app`) can use, in plain Node: Vitest's module runner reads an import
 * that isn't initialized yet as `undefined` instead of throwing.
 */
function directiveNames(dir: string, first: string, cmp = 'C'): string {
  const script = `
    globalThis.ngJitMode = false;
    globalThis.ngDevMode = false;
    await import(${JSON.stringify(pathToFileURL(join(dir, `${first}.mjs`)).href)});
    const app = await import('./app.mjs');
    const defs = app.${cmp}.ɵcmp.directiveDefs;
    const list = typeof defs === 'function' ? defs() : (defs ?? []);
    console.log(list.map((def) => def.type.name).sort().join(','));
  `
  return execFileSync(process.execPath, ['--input-type=module', '-e', script], {
    cwd: dir,
    encoding: 'utf8',
    stdio: ['ignore', 'pipe', 'pipe'],
  }).trim()
}

const DEP = `
  import { Directive, NgModule } from '@angular/core';
  @Directive({ selector: '[d]', standalone: false }) export class D {}
  @Directive({ selector: '[o]', standalone: false }) export class O {}
  @NgModule({ declarations: [D], exports: [D] })
  export class DepModule {
    static forRoot() { return { ngModule: DepModule, providers: [] }; }
  }
  @NgModule({ declarations: [O], exports: [O] }) export class OtherModule {}
  export const DEP_LIST = [DepModule];
`

describe('NgModule scope read across files, with ngJitMode = false', () => {
  it('gives a declared component the directives of the modules a list gives', () => {
    const { dir, code } = build({
      dep: DEP,
      app: `
        import { Component, NgModule, forwardRef } from '@angular/core';
        import * as dep from './dep';
        import { DEP_LIST } from './dep';
        @Component({ selector: 'c', template: '<i d o></i>', standalone: false }) export class C {}
        @NgModule({
          declarations: [C],
          imports: [forwardRef(() => dep.OtherModule), ...DEP_LIST, dep.DepModule.forRoot()],
        })
        export class M {}
      `,
    })
    expect(code.app.replace(/\s+/g, '')).toContain(
      'ɵɵsetNgModuleScope(M,{declarations:[C],imports:[i1.OtherModule,i1.DepModule,i1.DepModule]})',
    )
    expect(directiveNames(dir, 'app')).toBe('C,D,O')
  })

  it('keeps a list with an entry it cannot read as written, next to a read one', () => {
    const { dir, code } = build({
      dep: DEP,
      app: `
        import { Component, Directive, NgModule } from '@angular/core';
        import * as dep from './dep';
        function other() { return dep.OtherModule; }
        @Component({ selector: 'c', template: '<i d o l></i>', standalone: false }) export class C {}
        @NgModule({ declarations: [C, Later], imports: [...dep.DEP_LIST, other()] })
        export class M {}
        @Directive({ selector: '[l]', standalone: false }) export class Later {}
      `,
    })
    expect(code.app.replace(/\s+/g, '')).toContain(
      'ɵɵsetNgModuleScope(M,{declarations:()=>[C,Later],imports:[...dep.DEP_LIST,other()]})',
    )
    expect(directiveNames(dir, 'app')).toBe('C,D,Later,O')
  })

  it('keeps the lists lazy when the file of a listed class loads this one back', () => {
    const files = {
      cyc: `
        import { Directive, NgModule } from '@angular/core';
        import { M } from './app';
        @Directive({ selector: '[x]', standalone: false }) export class X {}
        @NgModule({ declarations: [X], exports: [X] })
        export class CycModule { static app = () => M; }
      `,
      app: `
        import { Component, NgModule, forwardRef } from '@angular/core';
        import * as c from './cyc';
        @Component({ selector: 'c', template: '<i x></i>', standalone: false }) export class C {}
        @NgModule({ declarations: [C], imports: [forwardRef(() => c.CycModule)] })
        export class M {}
      `,
    }
    const { dir, code } = build(files)
    const scope = 'ɵɵsetNgModuleScope(M,{declarations:()=>[C],imports:()=>[i1.CycModule]})'
    expect(code.app.replace(/\s+/g, '')).toContain(scope)
    // Loaded either way round: from `cyc`, `app` runs before `CycModule`
    // is defined.
    for (const first of ['app', 'cyc']) {
      expect(directiveNames(dir, first)).toBe('C,X')
    }
    // ngtsc writes the lists without the wrap here, which throws when
    // `cyc` is loaded first.
    const eager = code.app.replace(/\(\)\s*=>\s*\[i1\.CycModule\]/, '[i1.CycModule]')
    expect(eager).not.toBe(code.app)
    writeMjs(dir, 'app', eager, Object.keys(files))
    expect(directiveNames(dir, 'app')).toBe('C,X')
    expect(() => directiveNames(dir, 'cyc')).toThrow(/before initialization/)
  })
})

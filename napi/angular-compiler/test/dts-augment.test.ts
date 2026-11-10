import { execFileSync } from 'node:child_process'
import {
  cpSync,
  mkdirSync,
  mkdtempSync,
  readFileSync,
  readdirSync,
  realpathSync,
  rmSync,
  symlinkSync,
  writeFileSync,
} from 'node:fs'
import { createRequire } from 'node:module'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'

import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { augmentDeclarations } from '../vite-plugin/dts-augment.js'

const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const tscBin = join(
  dirname(createRequire(import.meta.url).resolve('typescript/package.json')),
  'bin/tsc',
)

// The library: every file reached from the entry, which is the only file
// the tsconfig lists (`tsc` still writes a `.d.ts` for each of them).
const SOURCES: Record<string, string> = {
  'src/public-api.ts': `export * from './lib/foo.component';
export * from './lib/bar.directive';
export * from './lib/upper.pipe';
export * from './lib/foo.module';
export * from './lib/sel.directive';
export * from './lib/util';
`,
  'src/lib/bar.directive.ts': `import { Directive, Input, HostBinding } from '@angular/core';

@Directive({ selector: '[libBar]', standalone: true })
export class BarDirective {
  @Input() barColor = 'red';
  @HostBinding('style.color') get color() { return this.barColor; }
}
`,
  'src/lib/upper.pipe.ts': `import { Pipe, PipeTransform } from '@angular/core';

@Pipe({ name: 'upper' })
export class UpperPipe implements PipeTransform {
  transform(v: string): string { return v.toUpperCase(); }
}
`,
  'src/lib/foo.component.ts': `import { Component, Input, input, output, booleanAttribute } from '@angular/core';

@Component({
  selector: 'lib-foo',
  templateUrl: './foo.component.html',
  exportAs: 'libFoo',
})
export class FooComponent {
  @Input({ transform: booleanAttribute }) disabled = false;
  @Input({ required: true }) label!: string;
  readonly size = input<number>(1);
  readonly changed = output<string>();
}
`,
  'src/lib/foo.component.html': `<span>{{ label }}</span>\n`,
  'src/lib/foo.module.ts': `import { NgModule } from '@angular/core';
import { FooComponent } from './foo.component';
import { UpperPipe } from './upper.pipe';

@NgModule({ imports: [FooComponent, UpperPipe], exports: [FooComponent, UpperPipe] })
export class FooModule {}
`,
  'src/lib/consts.ts': `export const SEL: string = '[libSel]';\n`,
  'src/lib/sel.directive.ts': `import { Directive } from '@angular/core';
import { SEL } from './consts';

@Directive({ selector: SEL })
export class SelDirective {}
`,
  'src/lib/util.ts': `export class Helper {
  x = 1;
}
`,
}

const TSCONFIG = {
  compilerOptions: {
    target: 'ES2022',
    module: 'ES2022',
    moduleResolution: 'bundler',
    rootDir: 'src',
    outDir: 'dist',
    declaration: true,
    declarationMap: true,
    skipLibCheck: true,
    strict: true,
    experimentalDecorators: true,
    useDefineForClassFields: false,
    lib: ['es2022', 'dom'],
  },
  angularCompilerOptions: { compilationMode: 'partial' },
  files: ['src/public-api.ts'],
}

// What ngc 22.2 (`ngc -p tsconfig.json`) writes for the library above.
const NGC: Record<string, string> = {
  'lib/bar.directive.d.ts': `import * as i0 from "@angular/core";
export declare class BarDirective {
    barColor: string;
    get color(): string;
    static ɵfac: i0.ɵɵFactoryDeclaration<BarDirective, never>;
    static ɵdir: i0.ɵɵDirectiveDeclaration<BarDirective, "[libBar]", never, { "barColor": { "alias": "barColor"; "required": false; }; }, {}, never, never, true, never>;
}
//# sourceMappingURL=bar.directive.d.ts.map`,
  'lib/consts.d.ts': `export declare const SEL: string;
//# sourceMappingURL=consts.d.ts.map`,
  'lib/foo.component.d.ts': `import * as i0 from "@angular/core";
export declare class FooComponent {
    disabled: boolean;
    label: string;
    readonly size: import("@angular/core").InputSignal<number>;
    readonly changed: import("@angular/core").OutputEmitterRef<string>;
    static ɵfac: i0.ɵɵFactoryDeclaration<FooComponent, never>;
    static ɵcmp: i0.ɵɵComponentDeclaration<FooComponent, "lib-foo", ["libFoo"], { "disabled": { "alias": "disabled"; "required": false; }; "label": { "alias": "label"; "required": true; }; "size": { "alias": "size"; "required": false; "isSignal": true; }; }, { "changed": "changed"; }, never, never, true, never>;
    static ngAcceptInputType_disabled: unknown;
}
//# sourceMappingURL=foo.component.d.ts.map`,
  'lib/foo.module.d.ts': `import * as i0 from "@angular/core";
import * as i1 from "./foo.component";
import * as i2 from "./upper.pipe";
export declare class FooModule {
    static ɵfac: i0.ɵɵFactoryDeclaration<FooModule, never>;
    static ɵmod: i0.ɵɵNgModuleDeclaration<FooModule, never, [typeof i1.FooComponent, typeof i2.UpperPipe], [typeof i1.FooComponent, typeof i2.UpperPipe]>;
    static ɵinj: i0.ɵɵInjectorDeclaration<FooModule>;
}
//# sourceMappingURL=foo.module.d.ts.map`,
  'lib/sel.directive.d.ts': `import * as i0 from "@angular/core";
export declare class SelDirective {
    static ɵfac: i0.ɵɵFactoryDeclaration<SelDirective, never>;
    static ɵdir: i0.ɵɵDirectiveDeclaration<SelDirective, "[libSel]", never, {}, {}, never, never, true, never>;
}
//# sourceMappingURL=sel.directive.d.ts.map`,
  'lib/upper.pipe.d.ts': `import { PipeTransform } from '@angular/core';
import * as i0 from "@angular/core";
export declare class UpperPipe implements PipeTransform {
    transform(v: string): string;
    static ɵfac: i0.ɵɵFactoryDeclaration<UpperPipe, never>;
    static ɵpipe: i0.ɵɵPipeDeclaration<UpperPipe, "upper", true>;
}
//# sourceMappingURL=upper.pipe.d.ts.map`,
  'lib/util.d.ts': `export declare class Helper {
    x: number;
}
//# sourceMappingURL=util.d.ts.map`,
  'public-api.d.ts': `export * from './lib/foo.component';
export * from './lib/bar.directive';
export * from './lib/upper.pipe';
export * from './lib/foo.module';
export * from './lib/sel.directive';
export * from './lib/util';
//# sourceMappingURL=public-api.d.ts.map`,
}

// `tsc`'s map of `lib/bar.directive.d.ts`, and the same map once the import
// line and the two members are in (original tokens keep their mapping).
const TSC_BAR_MAPPINGS = 'AAEA,qBACa,YAAY;IACd,QAAQ,SAAS;IAC1B,IAAgC,KAAK,WAA4B;CAClE'
const AUGMENTED_BAR_MAPPINGS = ';AAEA,qBACa,YAAY;IACd,QAAQ,SAAS;IAC1B,IAAgC,KAAK,WAA4B;;;CAClE'

const ANGULAR_FILES = [
  'lib/bar.directive.d.ts',
  'lib/foo.component.d.ts',
  'lib/foo.module.d.ts',
  'lib/sel.directive.d.ts',
  'lib/upper.pipe.d.ts',
]

let root: string

/** A fresh copy of the library in `root/<name>`. */
function project(name: string): string {
  const dir = join(root, name)
  for (const [file, text] of Object.entries(SOURCES)) {
    mkdirSync(dirname(join(dir, file)), { recursive: true })
    writeFileSync(join(dir, file), text)
  }
  writeFileSync(join(dir, 'tsconfig.json'), JSON.stringify(TSCONFIG, null, 2))
  return dir
}

function tsc(dir: string, ...args: string[]): void {
  try {
    execFileSync(process.execPath, [tscBin, ...args], { cwd: dir, stdio: 'pipe', encoding: 'utf8' })
  } catch (error) {
    const { stdout = '', stderr = '' } = error as { stdout?: string; stderr?: string }
    throw new Error(`tsc ${args.join(' ')} failed in ${dir}:\n${stdout}${stderr}`)
  }
}

function read(file: string): string {
  return readFileSync(file, 'utf8')
}

/** The path of `file` from `dir`, with `/` on every platform. */
function rel(dir: string, file: string): string {
  return relative(dir, file).split(sep).join('/')
}

/** Every file under `dir`, by path from `dir`, with its text. */
function snapshot(dir: string): Record<string, string> {
  const out: Record<string, string> = {}
  for (const entry of readdirSync(dir, { recursive: true, withFileTypes: true })) {
    if (!entry.isFile()) continue
    const file = join(entry.parentPath, entry.name)
    out[rel(dir, file)] = read(file)
  }
  return out
}

function mappings(file: string): string {
  return JSON.parse(read(file)).mappings
}

beforeAll(() => {
  // `native` also expands a Windows short name (`C:\Users\RUNNER~1`), so
  // tsc sees one spelling of every path.
  root = realpathSync.native(mkdtempSync(join(tmpdir(), 'oxc-angular-dts-augment-')))
  // `typescript` and `@angular/core` for the projects, as a library has them.
  symlinkSync(join(packageDir, 'node_modules'), join(root, 'node_modules'), 'junction')
})

afterAll(() => {
  rmSync(root, { recursive: true, force: true })
})

describe('augmentDeclarations with a tsconfig', () => {
  it("gives ngc's declaration files for a tsc build, also for files only reached by imports", async () => {
    const dir = project('basic')
    tsc(dir, '-p', 'tsconfig.json')
    const dist = join(dir, 'dist')
    expect(mappings(join(dist, 'lib/bar.directive.d.ts.map'))).toBe(TSC_BAR_MAPPINGS)
    const plain = snapshot(dist)

    const result = await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })

    expect(result.errors).toEqual([])
    expect(result.missing).toEqual([])
    expect(result.unmatched).toEqual([])
    expect(result.written.map((f) => rel(dist, f)).sort()).toEqual(ANGULAR_FILES)
    expect(result.matched).toHaveLength(Object.keys(NGC).length)
    for (const [file, text] of Object.entries(NGC)) {
      expect(read(join(dist, file)), file).toBe(text)
    }
    // Files without Angular classes are left as tsc wrote them.
    expect(read(join(dist, 'lib/util.d.ts'))).toBe(plain['lib/util.d.ts'])
    expect(read(join(dist, 'public-api.d.ts'))).toBe(plain['public-api.d.ts'])
    // The map is shifted to fit the new lines.
    expect(mappings(join(dist, 'lib/bar.directive.d.ts.map'))).toBe(AUGMENTED_BAR_MAPPINGS)

    // A second run changes nothing.
    const once = snapshot(dist)
    const again = await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })
    expect(again.written).toEqual([])
    expect(again.errors).toEqual([])
    expect(snapshot(dist)).toEqual(once)
  })

  it('takes options given to tsc on the command line, relative to cwd', async () => {
    const dir = project('cli')
    const { declaration, declarationMap, outDir, ...rest } = TSCONFIG.compilerOptions
    expect([declaration, declarationMap, outDir]).toEqual([true, true, 'dist'])
    writeFileSync(
      join(dir, 'tsconfig.json'),
      JSON.stringify({ ...TSCONFIG, compilerOptions: rest }, null, 2),
    )
    tsc(dir, '-p', 'tsconfig.json', '--declaration', '--emitDeclarationOnly', '--outDir', 'types')

    const result = await augmentDeclarations({
      tsconfig: 'tsconfig.json',
      compilerOptions: { outDir: 'types' },
      cwd: dir,
    })

    expect(result.errors).toEqual([])
    expect(result.missing).toEqual([])
    expect(result.written).toHaveLength(ANGULAR_FILES.length)
    for (const [file, text] of Object.entries(NGC)) {
      expect(read(join(dir, 'types', file)), file).toBe(
        text.replace(/\n\/\/# sourceMappingURL=.*$/, '\n'),
      )
    }

    // `declarationDir` is the same as `compilerOptions.declarationDir`.
    tsc(
      dir,
      '-p',
      'tsconfig.json',
      '--declaration',
      '--emitDeclarationOnly',
      '--declarationDir',
      'dd',
    )
    const byDir = await augmentDeclarations({
      tsconfig: 'tsconfig.json',
      declarationDir: 'dd',
      cwd: dir,
    })
    expect(byDir.written.map((f) => rel(join(dir, 'dd'), f)).sort()).toEqual(ANGULAR_FILES)
  })

  it('keeps CRLF line endings', async () => {
    const dir = project('crlf')
    tsc(dir, '-p', 'tsconfig.json', '--newLine', 'crlf')

    const result = await augmentDeclarations({
      tsconfig: join(dir, 'tsconfig.json'),
      compilerOptions: { newLine: 'crlf' },
    })

    expect(result.errors).toEqual([])
    // ngc writes the same text with `\r\n`.
    for (const [file, text] of Object.entries(NGC)) {
      expect(read(join(dir, 'dist', file)), file).toBe(text.replace(/\n/g, '\r\n'))
    }
    expect(mappings(join(dir, 'dist/lib/bar.directive.d.ts.map'))).toBe(AUGMENTED_BAR_MAPPINGS)
  })

  it('keeps CRLF when newLine was only given to tsc, and leaves plain files alone', async () => {
    const dir = project('crlf-cli')
    tsc(dir, '-p', 'tsconfig.json', '--newLine', 'crlf')
    const dist = join(dir, 'dist')
    const plain = snapshot(dist)

    const result = await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })

    expect(result.errors).toEqual([])
    expect(result.written.map((f) => rel(dist, f)).sort()).toEqual(ANGULAR_FILES)
    for (const [file, text] of Object.entries(NGC)) {
      expect(read(join(dist, file)), file).toBe(text.replace(/\n/g, '\r\n'))
    }
    expect(read(join(dist, 'lib/util.d.ts'))).toBe(plain['lib/util.d.ts'])
    expect(mappings(join(dist, 'lib/bar.directive.d.ts.map'))).toBe(AUGMENTED_BAR_MAPPINGS)
  })

  it('keeps the line breaks inside template literal types as they are', async () => {
    const dir = project('template-literal')
    // tsc copies these line breaks from the source: `\r\n` in one file,
    // `\n` in the other, whatever `newLine` is.
    writeFileSync(
      join(dir, 'src/lib/crlf.ts'),
      [
        'import { Directive } from "@angular/core";',
        'export type W = `x\r\ny`;',
        '@Directive({ selector: "[libW]" })',
        'export class WDir {}',
        '',
      ].join('\r\n'),
    )
    writeFileSync(
      join(dir, 'src/lib/lf.ts'),
      "import { Directive } from '@angular/core';\nexport type T = `x\ny`;\n" +
        "@Directive({ selector: '[libT]' })\nexport class TDir {}\n",
    )
    writeFileSync(
      join(dir, 'src/public-api.ts'),
      "export * from './lib/crlf';\nexport * from './lib/lf';\n",
    )
    tsc(dir, '-p', 'tsconfig.json')
    tsc(dir, '-p', 'tsconfig.json', '--newLine', 'crlf', '--outDir', 'dist-crlf')

    await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })
    await augmentDeclarations({
      tsconfig: join(dir, 'tsconfig.json'),
      compilerOptions: { newLine: 'crlf', outDir: 'dist-crlf' },
      cwd: dir,
    })

    // What ngc writes, with `\r\n` and `\n` in the template literal types.
    const ngc = (name: string, selector: string, type: string, file: string) => [
      'import * as i0 from "@angular/core";',
      type,
      `export declare class ${name} {`,
      `    static ɵfac: i0.ɵɵFactoryDeclaration<${name}, never>;`,
      `    static ɵdir: i0.ɵɵDirectiveDeclaration<${name}, "${selector}", never, {}, {}, never, never, true, never>;`,
      '}',
      `//# sourceMappingURL=${file}.d.ts.map`,
    ]
    const w = ngc('WDir', '[libW]', 'export type W = `x\r\ny`;', 'crlf')
    const t = ngc('TDir', '[libT]', 'export type T = `x\ny`;', 'lf')
    expect(read(join(dir, 'dist/lib/crlf.d.ts'))).toBe(w.join('\n'))
    expect(read(join(dir, 'dist/lib/lf.d.ts'))).toBe(t.join('\n'))
    expect(read(join(dir, 'dist-crlf/lib/crlf.d.ts'))).toBe(w.join('\r\n'))
    expect(read(join(dir, 'dist-crlf/lib/lf.d.ts'))).toBe(t.join('\r\n'))
  })

  it('shifts the map when declarationMap was only given to tsc', async () => {
    const dir = project('map-cli')
    const { declarationMap, ...rest } = TSCONFIG.compilerOptions
    expect(declarationMap).toBe(true)
    writeFileSync(
      join(dir, 'tsconfig.json'),
      JSON.stringify({ ...TSCONFIG, compilerOptions: rest }, null, 2),
    )
    tsc(dir, '-p', 'tsconfig.json', '--declarationMap')
    const map = join(dir, 'dist/lib/bar.directive.d.ts.map')
    expect(mappings(map)).toBe(TSC_BAR_MAPPINGS)

    await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })

    expect(read(join(dir, 'dist/lib/bar.directive.d.ts'))).toBe(NGC['lib/bar.directive.d.ts'])
    expect(mappings(map)).toBe(AUGMENTED_BAR_MAPPINGS)
  })

  it('keeps the byte order mark of emitBOM', async () => {
    const dir = project('bom')
    tsc(dir, '-p', 'tsconfig.json', '--emitBOM')

    const result = await augmentDeclarations({
      tsconfig: join(dir, 'tsconfig.json'),
      compilerOptions: { emitBOM: true },
    })

    expect(result.written).toHaveLength(ANGULAR_FILES.length)
    // ngc writes the same text after the mark.
    const bom = String.fromCharCode(0xfeff)
    for (const [file, text] of Object.entries(NGC)) {
      expect(read(join(dir, 'dist', file)), file).toBe(bom + text)
    }
  })

  it('shifts the map when mapRoot makes the map comment a URL', async () => {
    const dir = project('map-root')
    tsc(dir, '-p', 'tsconfig.json', '--mapRoot', 'https://example.com/maps/')
    const plain = snapshot(join(dir, 'dist'))

    // With `mapRoot` the maps name their sources by absolute file URL, and
    // `tsc` saw the real path. A path through a link gives other URLs, so
    // the maps `tsc` wrote must be kept.
    const link = join(root, 'map-root-link')
    symlinkSync(dir, link, 'junction')
    await augmentDeclarations({
      tsconfig: join(link, 'tsconfig.json'),
      compilerOptions: { mapRoot: 'https://example.com/maps/' },
    })
    expect(read(join(dir, 'dist/lib/util.d.ts.map'))).toBe(plain['lib/util.d.ts.map'])
    const sources = (file: string) => JSON.parse(file).sources
    expect(sources(read(join(dir, 'dist/lib/bar.directive.d.ts.map')))).toEqual(
      sources(plain['lib/bar.directive.d.ts.map']),
    )

    const bar = read(join(dir, 'dist/lib/bar.directive.d.ts'))
    expect(bar).toBe(
      NGC['lib/bar.directive.d.ts'].replace(
        'sourceMappingURL=',
        'sourceMappingURL=https://example.com/maps/lib/',
      ),
    )
    expect(mappings(join(dir, 'dist/lib/bar.directive.d.ts.map'))).toBe(AUGMENTED_BAR_MAPPINGS)
  })

  it('replaces members an incremental tsc run left in place', async () => {
    const dir = project('incremental')
    const tsconfig = join(dir, 'tsconfig.json')
    writeFileSync(
      tsconfig,
      JSON.stringify({
        ...TSCONFIG,
        compilerOptions: {
          ...TSCONFIG.compilerOptions,
          incremental: true,
          tsBuildInfoFile: 'dist/.tsbuildinfo',
        },
      }),
    )
    tsc(dir, '-p', 'tsconfig.json')
    await augmentDeclarations({ tsconfig })
    const sel = join(dir, 'dist/lib/sel.directive.d.ts')
    expect(read(sel)).toBe(NGC['lib/sel.directive.d.ts'])

    // The selector's type stays `string`, so tsc does not write the
    // directive's file again: its members still say `[libSel]`.
    writeFileSync(join(dir, 'src/lib/consts.ts'), `export const SEL: string = '[other]';\n`)
    tsc(dir, '-p', 'tsconfig.json')
    expect(read(sel)).toBe(NGC['lib/sel.directive.d.ts'])

    const result = await augmentDeclarations({ tsconfig })
    expect(result.written).toEqual([sel])
    // ngc writes `"[other]"`.
    expect(read(sel)).toBe(NGC['lib/sel.directive.d.ts'].replace('"[libSel]"', '"[other]"'))
  })

  it('lists the declaration files that are not there yet', async () => {
    const dir = project('missing')
    const result = await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })
    expect(result.written).toEqual([])
    expect(result.matched).toEqual([])
    expect(result.missing.map((f) => rel(join(dir, 'dist'), f)).sort()).toEqual(
      Object.keys(NGC).sort(),
    )
  })

  it('reports compile errors and leaves that file alone', async () => {
    const dir = project('error')
    rmSync(join(dir, 'src/lib/foo.component.html'))
    tsc(dir, '-p', 'tsconfig.json')
    const plain = read(join(dir, 'dist/lib/foo.component.d.ts'))

    const result = await augmentDeclarations({ tsconfig: join(dir, 'tsconfig.json') })

    expect(result.errors).toHaveLength(1)
    expect(result.errors[0].file).toBe(join(dir, 'src/lib/foo.component.ts'))
    expect(result.errors[0].message).toContain('COMPONENT_RESOURCE_NOT_FOUND')
    expect(read(join(dir, 'dist/lib/foo.component.d.ts'))).toBe(plain)
    expect(read(join(dir, 'dist/lib/bar.directive.d.ts'))).toBe(NGC['lib/bar.directive.d.ts'])
  })
})

describe('augmentDeclarations with sources', () => {
  const sources = Object.keys(SOURCES).filter((file) => file.endsWith('.ts'))

  it('matches files without maps by the layout tsc used, without TypeScript', async () => {
    const dir = project('sources')
    tsc(dir, '-p', 'tsconfig.json', '--declarationMap', 'false')
    // A file whose source is not one of the sources: never touched.
    const stray = join(dir, 'dist/testing/bar.directive.d.ts')
    mkdirSync(dirname(stray), { recursive: true })
    cpSync(join(dir, 'dist/lib/bar.directive.d.ts'), stray)
    const strayText = read(stray)

    const result = await augmentDeclarations({ declarationDir: 'dist', sources, cwd: dir })

    expect(result.errors).toEqual([])
    expect(result.unmatched).toEqual([stray])
    expect(result.written.map((f) => rel(join(dir, 'dist'), f)).sort()).toEqual(ANGULAR_FILES)
    for (const [file, text] of Object.entries(NGC)) {
      expect(read(join(dir, 'dist', file)), file).toBe(
        text.replace(/\n\/\/# sourceMappingURL=.*$/, '\n'),
      )
    }
    expect(read(stray)).toBe(strayText)

    const again = await augmentDeclarations({ declarationDir: 'dist', sources, cwd: dir })
    expect(again.written).toEqual([])
  })

  it('compiles with the tsconfig when one is given (paths aliases)', async () => {
    const dir = project('paths')
    const tsconfig = {
      ...TSCONFIG,
      compilerOptions: { ...TSCONFIG.compilerOptions, paths: { '@lib/*': ['./src/lib/*'] } },
    }
    writeFileSync(join(dir, 'tsconfig.json'), JSON.stringify(tsconfig))
    const selFile = join(dir, 'src/lib/sel.directive.ts')
    writeFileSync(selFile, read(selFile).replace("'./consts'", "'@lib/consts'"))
    tsc(dir, '-p', 'tsconfig.json')
    const plain = read(join(dir, 'dist/lib/sel.directive.d.ts'))

    // Without the tsconfig the alias can't be followed: an error, and the
    // file keeps no wrong members.
    const without = await augmentDeclarations({ declarationDir: 'dist', sources, cwd: dir })
    expect(without.errors.map((e) => e.file)).toEqual([selFile])
    expect(read(join(dir, 'dist/lib/sel.directive.d.ts'))).toBe(plain)

    const withTsconfig = await augmentDeclarations({
      tsconfig: 'tsconfig.json',
      declarationDir: 'dist',
      sources,
      cwd: dir,
    })
    expect(withTsconfig.errors).toEqual([])
    expect(read(join(dir, 'dist/lib/sel.directive.d.ts'))).toBe(NGC['lib/sel.directive.d.ts'])
  })

  it('needs declarationDir', async () => {
    await expect(augmentDeclarations({ sources })).rejects.toThrow(/declarationDir/)
    await expect(augmentDeclarations({})).rejects.toThrow(/tsconfig/)
  })

  it('fails on a declarationDir that is missing or holds no declarations', async () => {
    const dir = mkdtempSync(join(tmpdir(), 'dts-augment-dir-'))
    try {
      await expect(
        augmentDeclarations({ declarationDir: 'dist', sources, cwd: dir }),
      ).rejects.toThrow(/is not a directory/)
      mkdirSync(join(dir, 'dist'))
      await expect(
        augmentDeclarations({ declarationDir: 'dist', sources, cwd: dir }),
      ).rejects.toThrow(/holds no declaration files/)
    } finally {
      rmSync(dir, { recursive: true, force: true })
    }
  })
})

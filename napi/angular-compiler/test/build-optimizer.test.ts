import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'

import { normalizePath, resolveConfig, type InlineConfig } from 'vite'
import { afterAll, beforeAll, describe, expect, it } from 'vitest'

import { buildOptimizerPlugin } from '../vite-plugin/angular-build-optimizer-plugin.js'

let root: string

// `packageJson`, when given, is written as is and `sideEffects` is not used.
function writePackage(
  name: string,
  sideEffects: unknown,
  files: Record<string, string>,
  packageJson = JSON.stringify({ name, sideEffects }),
) {
  const dir = join(root, 'node_modules', name)
  mkdirSync(dir, { recursive: true })
  writeFileSync(join(dir, 'package.json'), packageJson)
  for (const [file, code] of Object.entries(files)) {
    mkdirSync(dirname(join(dir, file)), { recursive: true })
    writeFileSync(join(dir, file), code)
  }
}

function createPlugin() {
  const plugin = buildOptimizerPlugin({ jit: false, sourcemap: false, thirdPartySourcemaps: false })
  ;(plugin as any).config({ mode: 'production' })
  return plugin
}

async function optimize(file: string, plugin = createPlugin()) {
  // Vite passes forward-slash ids to transform hooks, also on Windows.
  const id = normalizePath(join(root, 'node_modules', file))
  const result = await (plugin as any).transform.handler.call({}, readFileSync(id, 'utf-8'), id)
  return result.code as string
}

const ZONE = `var Zone2 = loadZone();
patchCommon(Zone2);
`
const PUBLISH_FACADE = `publishFacade(_global);
`
const TOKEN = `const TOKEN = new InjectionToken('TOKEN');
const VERSION = new Version('22.0.0');
`
const STATIC_MEMBERS = `let Service = class Service {};
Service.ɵfac = function Service_Factory(t) { return new (t || Service)(); };
export { Service };
`

beforeAll(() => {
  root = mkdtempSync(join(tmpdir(), 'build-optimizer-test-'))
  writePackage('zone.js', undefined, { 'fesm2015/zone.js': ZONE })
  writePackage('@angular/core', false, { 'fesm2022/core.mjs': TOKEN })
  writePackage('@angular/build', undefined, { 'fesm2022/build.mjs': TOKEN })
  writePackage('@angular/upgrade', true, { 'fesm2022/upgrade.mjs': TOKEN })
  writePackage('@angular/compiler', ['./fesm2022/compiler.mjs'], {
    'fesm2022/compiler.mjs': PUBLISH_FACADE,
    'fesm2022/other.mjs': TOKEN,
  })
  writePackage('@angular/localize', ['*.mjs'], { 'fesm2022/localize.mjs': PUBLISH_FACADE })
  writePackage('@angular/animations', [], { 'fesm2022/animations.mjs': TOKEN })
  writePackage('@angular/mixed', [false, './fesm2022/kept.mjs'], {
    'fesm2022/mixed.mjs': TOKEN,
    'fesm2022/kept.mjs': PUBLISH_FACADE,
  })
  writePackage('@ngrx/store', false, { 'fesm2022/ngrx-store.mjs': TOKEN })
  writePackage('side-effectful', true, { 'fesm2022/side-effectful.mjs': STATIC_MEMBERS })
  writePackage(
    '@angular/bom',
    undefined,
    { 'fesm2022/bom.mjs': TOKEN },
    `\uFEFF${JSON.stringify({ sideEffects: false })}`,
  )
  writePackage(
    '@angular/broken',
    undefined,
    { 'fesm2022/broken.mjs': TOKEN + STATIC_MEMBERS },
    '{ "sideEffects": false, }',
  )
})

afterAll(() => {
  rmSync(root, { recursive: true, force: true })
})

describe('build optimizer: package sideEffects', () => {
  it('keeps top-level calls of zone.js, which is not an @angular package', async () => {
    const code = await optimize('zone.js/fesm2015/zone.js')

    expect(code).not.toContain('__PURE__')
    expect(code).toContain('patchCommon(Zone2)')
  })

  it('marks top-level calls pure in a side-effect-free @angular package', async () => {
    const code = await optimize('@angular/core/fesm2022/core.mjs')

    expect(code).toContain("/* @__PURE__ */ new InjectionToken('TOKEN')")
    expect(code).toContain("/* @__PURE__ */ new Version('22.0.0')")
  })

  it('keeps top-level calls of a file listed in the sideEffects array', async () => {
    const code = await optimize('@angular/compiler/fesm2022/compiler.mjs')

    expect(code).not.toContain('__PURE__')
  })

  it('marks top-level calls pure in a file not listed in the sideEffects array', async () => {
    const code = await optimize('@angular/compiler/fesm2022/other.mjs')

    expect(code).toContain("/* @__PURE__ */ new InjectionToken('TOKEN')")
  })

  it('matches a sideEffects pattern without a slash in any directory', async () => {
    const code = await optimize('@angular/localize/fesm2022/localize.mjs')

    expect(code).not.toContain('__PURE__')
  })

  it('treats an empty sideEffects array as side-effect-free', async () => {
    const code = await optimize('@angular/animations/fesm2022/animations.mjs')

    expect(code).toContain("/* @__PURE__ */ new InjectionToken('TOKEN')")
  })

  it('only marks top-level calls pure in @angular packages, like the Angular CLI', async () => {
    const code = await optimize('@ngrx/store/fesm2022/ngrx-store.mjs')

    expect(code).not.toContain('__PURE__')
  })

  it('still wraps static members of packages with side effects', async () => {
    const code = await optimize('side-effectful/fesm2022/side-effectful.mjs')

    expect(code).toMatch(/let Service = \/\*\s*[@#]__PURE__\s*\*\/ \(\(\) => \{/)
  })

  it('keeps top-level calls of an @angular package without a sideEffects field', async () => {
    const code = await optimize('@angular/build/fesm2022/build.mjs')

    expect(code).not.toContain('__PURE__')
  })

  it('keeps top-level calls of an @angular package with sideEffects: true', async () => {
    const code = await optimize('@angular/upgrade/fesm2022/upgrade.mjs')

    expect(code).not.toContain('__PURE__')
  })

  it('ignores sideEffects array entries that are not strings', async () => {
    const code = await optimize('@angular/mixed/fesm2022/mixed.mjs')

    expect(code).toContain("/* @__PURE__ */ new InjectionToken('TOKEN')")
    expect(await optimize('@angular/mixed/fesm2022/kept.mjs')).not.toContain('__PURE__')
  })

  it('reads a package.json that starts with a BOM', async () => {
    const code = await optimize('@angular/bom/fesm2022/bom.mjs')

    expect(code).toContain("/* @__PURE__ */ new InjectionToken('TOKEN')")
  })

  it('treats an invalid package.json as having side effects and still runs the other passes', async () => {
    const code = await optimize('@angular/broken/fesm2022/broken.mjs')

    expect(code).not.toContain("/* @__PURE__ */ new InjectionToken('TOKEN')")
    expect(code).toMatch(/let Service = \/\*\s*[@#]__PURE__\s*\*\/ \(\(\) => \{/)
  })

  it('keeps each package and file separate when one plugin handles many files', async () => {
    const plugin = createPlugin()

    expect(await optimize('@angular/core/fesm2022/core.mjs', plugin)).toContain('__PURE__')
    expect(await optimize('@angular/compiler/fesm2022/compiler.mjs', plugin)).not.toContain(
      '__PURE__',
    )
    expect(await optimize('@angular/compiler/fesm2022/other.mjs', plugin)).toContain('__PURE__')
    expect(await optimize('@angular/build/fesm2022/build.mjs', plugin)).not.toContain('__PURE__')
  })
})

describe('build optimizer: ngServerMode', () => {
  function resolveBuildConfig(config: InlineConfig, mode = 'production') {
    return resolveConfig(
      {
        configFile: false,
        logLevel: 'silent',
        mode,
        plugins: [
          buildOptimizerPlugin({ jit: false, sourcemap: false, thirdPartySourcemaps: false }),
        ],
        ...config,
      },
      'build',
    )
  }

  // The `ngServerMode` each environment of the resolved build config defines.
  async function ngServerModes(config: InlineConfig, mode?: string) {
    const resolved = await resolveBuildConfig(config, mode)
    return Object.fromEntries(
      Object.entries(resolved.environments).map(([name, env]) => [name, env.define?.ngServerMode]),
    )
  }

  it('defines it per environment when one config builds the client and the server', async () => {
    expect(
      await ngServerModes({ environments: { client: {}, ssr: {}, edge: { consumer: 'server' } } }),
    ).toEqual({ client: 'false', ssr: 'true', edge: 'true' })
  })

  it('defines it as true for a separate `vite build --ssr`', async () => {
    expect(await ngServerModes({ build: { ssr: 'src/server.ts' } })).toMatchObject({ ssr: 'true' })
  })

  it('only defines it in server environments outside production', async () => {
    expect(await ngServerModes({ environments: { client: {}, ssr: {} } }, 'development')).toEqual({
      client: undefined,
      ssr: 'true',
    })
  })

  it('leaves it out of the defines all environments share', async () => {
    const resolved = await resolveBuildConfig({})
    expect(resolved.define).not.toHaveProperty('ngServerMode')
    expect(resolved.oxc && resolved.oxc.define).not.toHaveProperty('ngServerMode')
  })
})

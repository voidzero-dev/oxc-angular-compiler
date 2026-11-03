import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import { normalizePath } from 'vite'
import { afterEach, describe, expect, it } from 'vitest'

import { angular, type PluginOptions } from '../vite-plugin/index.js'
import {
  installedAngularVersion,
  parseAngularVersion,
} from '../vite-plugin/utils/angular-version.js'

const dirs: string[] = []

/** A project directory with `@angular/core` at `version` installed, if any. */
function project(version?: string): string {
  const dir = mkdtempSync(join(tmpdir(), 'angular-version-'))
  dirs.push(dir)
  if (version) install(dir, version)
  return dir
}

/** `@angular/core` at `version` in `dir/node_modules`. */
function install(dir: string, version: string): void {
  const core = join(dir, 'node_modules/@angular/core')
  mkdirSync(core, { recursive: true })
  writeFileSync(join(core, 'package.json'), JSON.stringify({ name: '@angular/core', version }))
}

afterEach(() => {
  for (const dir of dirs.splice(0)) rmSync(dir, { recursive: true, force: true })
})

describe('parseAngularVersion', () => {
  it('reads stable and prerelease versions', () => {
    expect(parseAngularVersion('22.1.3')).toEqual({ major: 22, minor: 1, patch: 3 })
    expect(parseAngularVersion('22.2.0-next.3')).toEqual({
      major: 22,
      minor: 2,
      patch: 0,
      prerelease: 'next.3',
    })
    expect(parseAngularVersion('21.0.0+sha-abc')).toEqual({ major: 21, minor: 0, patch: 0 })
  })

  it("leaves out a build of Angular's own repository and anything else", () => {
    expect(parseAngularVersion('0.0.0-PLACEHOLDER')).toBeUndefined()
    expect(parseAngularVersion('22')).toBeUndefined()
    expect(parseAngularVersion(undefined)).toBeUndefined()
  })
})

describe('installedAngularVersion', () => {
  it('finds @angular/core from a nested directory', () => {
    const root = project('21.2.1')
    const nested = join(root, 'apps/app')
    mkdirSync(nested, { recursive: true })
    expect(installedAngularVersion(nested)).toEqual({ major: 21, minor: 2, patch: 1 })
  })

  it('resolves a relative root first', () => {
    // `.` in an app whose packages are hoisted to the workspace above it.
    const root = project('21.2.1')
    const nested = join(root, 'apps/app')
    mkdirSync(nested, { recursive: true })
    const cwd = process.cwd()
    process.chdir(nested)
    try {
      expect(installedAngularVersion('.')).toEqual({ major: 21, minor: 2, patch: 1 })
    } finally {
      process.chdir(cwd)
    }
  })

  it('gives undefined without a readable package', () => {
    const root = project()
    mkdirSync(join(root, 'node_modules/@angular/core'), { recursive: true })
    writeFileSync(join(root, 'node_modules/@angular/core/package.json'), '{')
    expect(installedAngularVersion(root)).toBeUndefined()
  })
})

const SOURCE = `import { Component } from '@angular/core';
import { Cmp } from './cmp';

@Component({
  selector: 'app-root',
  template: '@defer { <cmp-a /> }',
  deferredImports: [Cmp],
})
export class AppComponent {}
`

/** The plugin's output for `SOURCE` in `root`, or the error it reports. */
async function transform(options: PluginOptions, root: string): Promise<string> {
  const plugin = angular(options).find((candidate) => candidate.name === '@oxc-angular/vite')
  if (!plugin?.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }
  const configResolved = plugin.configResolved as unknown as
    | { handler: (config: unknown) => void }
    | ((config: unknown) => void)
  const config = { root, build: {}, isProduction: false }
  if (typeof configResolved === 'function') configResolved(config)
  else configResolved?.handler(config)
  try {
    const result = await plugin.transform.handler.call(
      {
        error(message: string) {
          throw new Error(message)
        },
        warn() {},
      } as any,
      SOURCE,
      normalizePath(join(root, 'src/app.component.ts')),
    )
    return (result as { code: string }).code
  } catch (error) {
    return `ERROR ${(error as Error).message}`
  }
}

describe('angularVersion default', () => {
  it('compiles the array deferredImports of an Angular 22.1 project', async () => {
    const root = project('22.1.0')
    const code = await transform({ workspaceRoot: root }, root)
    expect(code).not.toContain('ERROR')
    expect(code).toContain('import("./cmp")')
  })

  it('reports the array deferredImports of an Angular 22.2 project', async () => {
    const root = project('22.2.0')
    const code = await transform({ workspaceRoot: root }, root)
    expect(code).toContain("'deferredImports' must be an object mapping")
  })

  it('prefers the angularVersion option over the installed version', async () => {
    const root = project('22.2.0')
    const code = await transform(
      { workspaceRoot: root, angularVersion: { major: 22, minor: 1, patch: 0 } },
      root,
    )
    expect(code).not.toContain('ERROR')
  })

  it("reads the version installed for Vite's root, not the workspace root", async () => {
    // An app with its own `@angular/core` in a workspace on a newer one.
    const workspace = project('22.2.0')
    const app = join(workspace, 'apps/old')
    install(app, '22.1.0')
    const code = await transform({ workspaceRoot: workspace }, app)
    expect(code).not.toContain('ERROR')
  })

  it('links dependencies for the same version', async () => {
    const workspace = project('22.2.0')
    const app = join(workspace, 'apps/old')
    install(app, '22.1.0')
    const linker = angular({ workspaceRoot: workspace }).find(
      (candidate) => candidate.name === '@oxc-angular/vite-linker',
    ) as any
    const config = linker.config({ root: app }, { command: 'build', mode: 'production' })
    expect(config.optimizeDeps.rolldownOptions.plugins[0].name).toBe('angular-linker@22.1.0')
  })
})

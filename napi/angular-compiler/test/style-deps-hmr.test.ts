/**
 * Tests HMR for transitive style dependencies.
 *
 * A component styleUrl compiled through a CSS preprocessor can pull in shared
 * files (Sass partials via `@use` / `@import` / `meta.load-css`, Less imports,
 * ...). `preprocessCSS` reports them in `deps`; the plugin must invalidate the
 * compiled style and dispatch component HMR when one of them changes, for every
 * component whose style is built on top of it.
 */
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'

import type { Plugin, ModuleNode, HmrContext } from 'vite'
import { resolveConfig } from 'vite'
import { afterAll, beforeAll, describe, it, expect, vi } from 'vitest'

import { angular } from '../vite-plugin/index.js'

let tempDir: string
let appDir: string
let sharedScssPath: string
let firstComponentPath: string
let secondComponentPath: string

const componentSource = (selector: string, styleUrl: string) => `
  import { Component } from '@angular/core';

  @Component({
    selector: '${selector}',
    template: '<h1>Hello</h1>',
    styleUrls: ['./${styleUrl}'],
  })
  export class AppComponent {}
`

beforeAll(() => {
  // realpath: Sass canonicalizes loaded URLs (macOS /var -> /private/var),
  // and watcher events use canonical paths too.
  tempDir = realpathSync(mkdtempSync(join(tmpdir(), 'style-deps-hmr-test-')))
  appDir = join(tempDir, 'src', 'app')
  mkdirSync(appDir, { recursive: true })

  sharedScssPath = join(appDir, '_shared.scss')
  firstComponentPath = join(appDir, 'first.component.ts')
  secondComponentPath = join(appDir, 'second.component.ts')

  writeFileSync(sharedScssPath, 'h1 { color: red; }')
  writeFileSync(join(appDir, 'first.component.scss'), "@use './shared';")
  writeFileSync(join(appDir, 'second.component.scss'), "@use './shared';")
  writeFileSync(firstComponentPath, componentSource('app-first', 'first.component.scss'))
  writeFileSync(secondComponentPath, componentSource('app-second', 'second.component.scss'))
})

afterAll(() => {
  rmSync(tempDir, { recursive: true, force: true })
})

function getAngularPlugin() {
  const plugin = angular({ liveReload: true }).find(
    (candidate) => candidate.name === '@oxc-angular/vite',
  )

  if (!plugin) {
    throw new Error('Failed to find @oxc-angular/vite plugin')
  }

  return plugin
}

function createMockServer() {
  const wsMessages: any[] = []

  return {
    watcher: {
      add: vi.fn(),
      unwatch: vi.fn(),
      on: vi.fn(),
      emit: vi.fn(),
    },
    ws: {
      send(msg: any) {
        wsMessages.push(msg)
      },
      on: vi.fn(),
    },
    moduleGraph: {
      getModuleById: vi.fn(() => null),
      invalidateModule: vi.fn(),
    },
    middlewares: {
      use: vi.fn(),
    },
    config: {
      root: tempDir,
    },
    _wsMessages: wsMessages,
  }
}

function createMockHmrContext(file: string, server: any, modules: ModuleNode[] = []): HmrContext {
  return {
    file,
    timestamp: Date.now(),
    modules,
    read: async () => '',
    server,
  } as HmrContext
}

async function callPluginHook<TArgs extends unknown[], TResult>(
  hook:
    | {
        handler: (...args: TArgs) => TResult
      }
    | ((...args: TArgs) => TResult)
    | undefined,
  ...args: TArgs
): Promise<TResult | undefined> {
  if (!hook) return undefined
  if (typeof hook === 'function') return hook(...args)
  return hook.handler(...args)
}

async function setupPluginWithServer(plugin: Plugin) {
  const mockServer = createMockServer()

  await callPluginHook(
    plugin.config as Plugin['config'],
    {} as any,
    {
      command: 'serve',
      mode: 'development',
    } as any,
  )

  // A real resolved config: preprocessCSS needs one to run Sass and report
  // the partials it loaded in `deps`.
  const resolved = await resolveConfig(
    { configFile: false, root: tempDir, logLevel: 'silent' },
    'serve',
  )
  await callPluginHook(plugin.configResolved as Plugin['configResolved'], resolved as any)

  if (typeof plugin.configureServer === 'function') {
    await (plugin.configureServer as Function)(mockServer)
  }

  ;(mockServer as any).__angularWatchTemplate = () => {}

  return mockServer
}

async function transformComponent(plugin: Plugin, source: string, path: string) {
  if (!plugin.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }

  await plugin.transform.handler.call(
    { error() {}, warn() {}, addWatchFile() {} } as any,
    source,
    path,
  )
}

function componentUpdateCount(server: any): number {
  return server._wsMessages.filter((msg: any) => msg?.event === 'angular:component-update').length
}

describe('handleHotUpdate for transitive style dependencies', () => {
  it('dispatches HMR to every component whose style uses a changed Sass partial', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    await transformComponent(
      plugin,
      componentSource('app-first', 'first.component.scss'),
      firstComponentPath,
    )
    await transformComponent(
      plugin,
      componentSource('app-second', 'second.component.scss'),
      secondComponentPath,
    )

    const ctx = createMockHmrContext(sharedScssPath, mockServer)
    const result = await (plugin.handleHotUpdate as Function).call(plugin, ctx)

    // Handled by the plugin: no modules left for Vite's default pipeline.
    expect(result).toEqual([])

    // Both owning components received a component-update event.
    const updates = mockServer._wsMessages.filter(
      (msg) => msg?.event === 'angular:component-update',
    )
    const updatedIds = updates.map((msg) => decodeURIComponent(msg.data.id))
    expect(updatedIds.some((id) => id.startsWith(firstComponentPath))).toBe(true)
    expect(updatedIds.some((id) => id.startsWith(secondComponentPath))).toBe(true)
  })

  it('dispatches exactly once per component for a transitive-only partial', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Guard: `_shared.scss` is only ever a transitive dep, never a direct
    // styleUrl. The shared-dep branch handles it; the direct-resource branch
    // must not dispatch a second time for the same components.
    await transformComponent(
      plugin,
      componentSource('app-first', 'first.component.scss'),
      firstComponentPath,
    )
    await transformComponent(
      plugin,
      componentSource('app-second', 'second.component.scss'),
      secondComponentPath,
    )

    await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(sharedScssPath, mockServer),
    )

    // Two owning components, one class each: exactly two events.
    expect(componentUpdateCount(mockServer)).toBe(2)
  })

  it('leaves untracked stylesheets to Vite', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    const untracked = join(appDir, 'not-a-dep.scss')
    writeFileSync(untracked, 'h2 { color: blue; }')
    const ctx = createMockHmrContext(untracked, mockServer)
    const result = await (plugin.handleHotUpdate as Function).call(plugin, ctx)

    expect(result).toBe(ctx.modules)
  })

  it('re-registers style deps when a style file switches its imports via HMR', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Dedicated fixtures so this test never interferes with the shared ones.
    const stylePath = join(appDir, 'switch.component.scss')
    const firstDepPath = join(appDir, '_switch-shared.scss')
    const secondDepPath = join(appDir, '_switch-other.scss')
    const componentPath = join(appDir, 'switch.component.ts')

    writeFileSync(firstDepPath, 'h1 { color: red; }')
    writeFileSync(secondDepPath, 'h2 { color: green; }')
    writeFileSync(stylePath, "@use './switch-shared';")
    writeFileSync(componentPath, componentSource('app-switch', 'switch.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-switch', 'switch.component.scss'),
      componentPath,
    )

    // Sanity: the initially registered dep is tracked.
    let result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(firstDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(1)

    // Dev edits the style to import a different partial; HMR rebuilds it.
    writeFileSync(stylePath, "@use './switch-other';")
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(stylePath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(2)

    // The newly imported partial must now be registered as a dep: editing it
    // dispatches a third update (previously it fell through to Vite).
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(secondDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(3)
  })

  it('re-registers nested style deps when a partial switches its own imports via HMR', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // style -> partial -> nested import. Dedicated fixtures so this test never
    // interferes with the shared ones.
    const stylePath = join(appDir, 'nested.component.scss')
    const partialPath = join(appDir, '_nested-a.scss')
    const firstDepPath = join(appDir, '_nested-x.scss')
    const secondDepPath = join(appDir, '_nested-y.scss')
    const componentPath = join(appDir, 'nested.component.ts')

    writeFileSync(firstDepPath, 'h1 { color: red; }')
    writeFileSync(secondDepPath, 'h2 { color: green; }')
    writeFileSync(partialPath, "@use './nested-x';")
    writeFileSync(stylePath, "@use './nested-a';")
    writeFileSync(componentPath, componentSource('app-nested', 'nested.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-nested', 'nested.component.scss'),
      componentPath,
    )

    // Sanity: the initially registered transitive dep is tracked.
    let result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(firstDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(1)

    // Dev edits the partial to import a different nested file; HMR rebuilds.
    writeFileSync(partialPath, "@use './nested-y';")
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(partialPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(2)

    // The newly imported nested file must now be registered as a dep: editing
    // it dispatches a third update (previously it fell through to Vite).
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(secondDepPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(3)
  })

  it('dispatches HMR for a style that is both a shared dep and a direct styleUrl', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // a.component.scss imports shared.component.scss; component B references
    // shared.component.scss directly as its styleUrl.
    const aStylePath = join(appDir, 'a.component.scss')
    const sharedStylePath = join(appDir, 'shared.component.scss')
    const aComponentPath = join(appDir, 'a.component.ts')
    const bComponentPath = join(appDir, 'b.component.ts')

    writeFileSync(aStylePath, "@use './shared.component';")
    writeFileSync(sharedStylePath, 'h1 { color: red; }')
    writeFileSync(aComponentPath, componentSource('app-a', 'a.component.scss'))
    writeFileSync(bComponentPath, componentSource('app-b', 'shared.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-b', 'shared.component.scss'),
      bComponentPath,
    )
    // Transform A last: its transitive dep on shared.component.scss must NOT
    // clobber B's direct-owner mapping in resourceToComponent (which is
    // single-owner). This order previously left B without updates.
    await transformComponent(plugin, componentSource('app-a', 'a.component.scss'), aComponentPath)

    const result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(sharedStylePath, mockServer),
    )
    expect(result).toEqual([])

    // Both roles updated: A via the shared-dep branch, B via the
    // direct-resource branch (previously the early return skipped B).
    const updates = mockServer._wsMessages.filter(
      (msg) => msg?.event === 'angular:component-update',
    )
    const updatedIds = updates.map((msg) => decodeURIComponent(msg.data.id))
    expect(updatedIds.some((id) => id.startsWith(aComponentPath))).toBe(true)
    expect(updatedIds.some((id) => id.startsWith(bComponentPath))).toBe(true)
  })

  it('registers style deps with the watcher on transform', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    await transformComponent(
      plugin,
      componentSource('app-first', 'first.component.scss'),
      firstComponentPath,
    )

    // The style and every preprocessor dep must be added to the watcher so
    // edits reach handleHotUpdate even outside the dev-server root. Compare
    // canonical paths: Sass resolves deps to long names on Windows while the
    // temp dir may carry 8.3 short names (e.g. RUNNER~1).
    const added = mockServer.watcher.add.mock.calls
      .flat()
      .map((p: string) => realpathSync.native(p))
    expect(added).toContain(realpathSync.native(join(appDir, 'first.component.scss')))
    expect(added).toContain(realpathSync.native(sharedScssPath))
  })

  it('refreshes deps for a direct style that initially failed to preprocess', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Dedicated fixtures so this test never interferes with the shared ones.
    const stylePath = join(appDir, 'broken.component.scss')
    const partialPath = join(appDir, '_broken-partial.scss')
    const componentPath = join(appDir, 'broken.component.ts')

    writeFileSync(partialPath, 'h1 { color: red; }')
    writeFileSync(stylePath, "@use './broken-missing';")
    writeFileSync(componentPath, componentSource('app-broken', 'broken.component.scss'))

    // Initial transform: the import is missing, so preprocessing fails and no
    // deps are registered.
    await transformComponent(
      plugin,
      componentSource('app-broken', 'broken.component.scss'),
      componentPath,
    )

    // Developer fixes the style to import an existing partial.
    writeFileSync(stylePath, "@use './broken-partial';")
    let result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(stylePath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(1)

    // The newly valid import must now be tracked: editing it dispatches.
    result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(partialPath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(2)
  })

  it('preserves Vite modules for partials also imported by global stylesheets', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    await transformComponent(
      plugin,
      componentSource('app-first', 'first.component.scss'),
      firstComponentPath,
    )

    // Simulate the partial also being imported by a global stylesheet, which
    // puts that stylesheet's module in Vite's HMR context.
    const globalModule = { id: join(appDir, 'styles.scss') } as ModuleNode
    const ctx = createMockHmrContext(sharedScssPath, mockServer, [globalModule])
    const result = await (plugin.handleHotUpdate as Function).call(plugin, ctx)

    // Component HMR is dispatched for the owning style...
    expect(componentUpdateCount(mockServer)).toBe(1)
    // ...and the global stylesheet's module survives for Vite's pipeline
    // (previously the handled branch returned [], starving it).
    expect(result).toBe(ctx.modules)
    expect(result).toContain(globalModule)
  })

  it('dispatches HMR to every component sharing a style that imports the changed partial', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Both components reference the same style file directly; it imports a
    // partial that the dev edits.
    const sharedStylePath = join(appDir, 'multi-owner.component.scss')
    const partialPath = join(appDir, '_multi-owner-partial.scss')
    const firstComponentPath = join(appDir, 'multi-a.component.ts')
    const secondComponentPath = join(appDir, 'multi-b.component.ts')

    writeFileSync(partialPath, 'h1 { color: red; }')
    writeFileSync(sharedStylePath, "@use './multi-owner-partial';")
    writeFileSync(firstComponentPath, componentSource('app-multi-a', 'multi-owner.component.scss'))
    writeFileSync(secondComponentPath, componentSource('app-multi-b', 'multi-owner.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-multi-a', 'multi-owner.component.scss'),
      firstComponentPath,
    )
    await transformComponent(
      plugin,
      componentSource('app-multi-b', 'multi-owner.component.scss'),
      secondComponentPath,
    )

    const result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(partialPath, mockServer),
    )
    expect(result).toEqual([])

    // Every component that uses the shared style receives an update
    // (previously only the last-transformed owner did).
    const updates = mockServer._wsMessages.filter(
      (msg) => msg?.event === 'angular:component-update',
    )
    const updatedIds = updates.map((msg) => decodeURIComponent(msg.data.id))
    expect(updatedIds.some((id) => id.startsWith(firstComponentPath))).toBe(true)
    expect(updatedIds.some((id) => id.startsWith(secondComponentPath))).toBe(true)
  })

  it('dispatches HMR to remaining owners when one owner switches styles', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // Both components use the same root style directly.
    const sharedStylePath = join(appDir, 'shared-root.component.scss')
    const aComponentPath = join(appDir, 'owner-a.component.ts')
    const bComponentPath = join(appDir, 'owner-b.component.ts')
    const bNewStylePath = join(appDir, 'owner-b-other.component.scss')

    writeFileSync(sharedStylePath, 'h1 { color: red; }')
    writeFileSync(bNewStylePath, 'h2 { color: blue; }')
    writeFileSync(aComponentPath, componentSource('app-owner-a', 'shared-root.component.scss'))
    writeFileSync(bComponentPath, componentSource('app-owner-b', 'shared-root.component.scss'))

    await transformComponent(
      plugin,
      componentSource('app-owner-a', 'shared-root.component.scss'),
      aComponentPath,
    )
    await transformComponent(
      plugin,
      componentSource('app-owner-b', 'shared-root.component.scss'),
      bComponentPath,
    )

    // B switches to a different style; its prune removes the single-valued
    // resourceToComponent entry for the shared style.
    writeFileSync(bComponentPath, componentSource('app-owner-b', 'owner-b-other.component.scss'))
    await transformComponent(
      plugin,
      componentSource('app-owner-b', 'owner-b-other.component.scss'),
      bComponentPath,
    )

    // Editing the shared root style must still update A (reachable via
    // styleComponentOwners even though resourceToComponent no longer has it).
    const result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(sharedStylePath, mockServer),
    )
    expect(result).toEqual([])

    const updates = mockServer._wsMessages.filter(
      (msg) => msg?.event === 'angular:component-update',
    )
    const updatedIds = updates.map((msg) => decodeURIComponent(msg.data.id))
    expect(updatedIds.some((id) => id.startsWith(aComponentPath))).toBe(true)
  })

  it('dispatches HMR for Stylus styles like other stylesheet languages', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // A .styl styleUrl: without the `stylus` package installed preprocessing
    // fails, but the file is still tracked as a direct style — what matters
    // here is that the HMR branch routes non-css/scss/sass/less extensions.
    const stylePath = join(appDir, 'styl.component.styl')
    const componentPath = join(appDir, 'styl.component.ts')

    writeFileSync(stylePath, 'h1\n  color red')
    writeFileSync(componentPath, componentSource('app-styl', 'styl.component.styl'))

    await transformComponent(
      plugin,
      componentSource('app-styl', 'styl.component.styl'),
      componentPath,
    )

    const result = await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(stylePath, mockServer),
    )
    expect(result).toEqual([])
    expect(componentUpdateCount(mockServer)).toBe(1)
  })
})

describe('handleHotUpdate for a resource reached through both dispatch paths', () => {
  it('dispatches once per class when a transitive dep is also a direct templateUrl', async () => {
    const plugin = getAngularPlugin()
    const mockServer = await setupPluginWithServer(plugin)

    // `_dual-role.scss` wears two hats at once:
    //   - transitive dep: `dual-importer.component.scss` @uses it, so it
    //     reaches components through the shared-dep branch.
    //   - direct templateUrl: a component renders it directly, so it also
    //     reaches components through the direct-resource branch.
    // Its contents must parse as BOTH Sass and an Angular template, so a
    // comment is the one body that satisfies both grammars (a CSS rule fails
    // the template parser; bare text fails Sass).
    const partialPath = join(appDir, '_dual-role.scss')
    const importerPath = join(appDir, 'dual-importer.component.scss')
    const bothRolesPath = join(appDir, 'dual-both.component.ts')
    const transitiveOnlyPath = join(appDir, 'dual-transitive-only.component.ts')

    writeFileSync(partialPath, '/* DUAL_ROLE_MARKER */')
    writeFileSync(importerPath, "@use './dual-role';")

    // One `.ts` owning BOTH paths: its style pulls the partial in
    // transitively, and its sibling uses the same partial as a template.
    const bothRolesSource = `
      import { Component } from '@angular/core';

      @Component({
        selector: 'app-dual-style',
        template: '<h1>Hello</h1>',
        styleUrls: ['./dual-importer.component.scss'],
      })
      export class DualStyleComponent {}

      @Component({
        selector: 'app-dual-template',
        templateUrl: './_dual-role.scss',
      })
      export class DualTemplateComponent {}
    `
    // A second owner reached ONLY through the shared-dep branch. Its update
    // proves that branch really ran, so the assertions below are not just the
    // template branch firing twice.
    const transitiveOnlySource = componentSource(
      'app-dual-transitive',
      'dual-importer.component.scss',
    )

    writeFileSync(bothRolesPath, bothRolesSource)
    writeFileSync(transitiveOnlyPath, transitiveOnlySource)

    await transformComponent(plugin, bothRolesSource, bothRolesPath)
    await transformComponent(plugin, transitiveOnlySource, transitiveOnlyPath)

    writeFileSync(partialPath, '/* DUAL_ROLE_MARKER edited */')
    await (plugin.handleHotUpdate as Function).call(
      plugin,
      createMockHmrContext(partialPath, mockServer),
    )

    const ids = mockServer._wsMessages
      .filter((msg: any) => msg?.event === 'angular:component-update')
      .map((msg: any) => decodeURIComponent(msg.data.id))
    const countOf = (id: string) => ids.filter((candidate: string) => candidate === id).length

    // The shared-dep branch ran: the transitive-only owner was updated.
    expect(countOf(`${transitiveOnlyPath}@AppComponent`)).toBe(1)

    // The dual-path owner is dispatched by the shared-dep branch and would be
    // dispatched again by the direct-template loop. `ws.send` is not
    // idempotent, so each class must still see exactly one event.
    expect(countOf(`${bothRolesPath}@DualStyleComponent`)).toBe(1)
    expect(countOf(`${bothRolesPath}@DualTemplateComponent`)).toBe(1)
  })
})

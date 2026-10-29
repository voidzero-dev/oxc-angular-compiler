import { describe, it, expect } from 'vitest'

import { linkAngularPackageSync, type AngularVersion } from '../index.js'
import { angularLinkerPlugin } from '../vite-plugin/angular-linker-plugin.js'
import { angular, type PluginOptions } from '../vite-plugin/index.js'

/**
 * Minimal Angular partial declaration fixtures that simulate the structure
 * of FESM bundle files (including Angular 21+ chunk files).
 * Uses actual Unicode ɵ (U+0275) characters as they appear in real Angular packages.
 */
const INJECTABLE_CHUNK = `
import * as i0 from '@angular/core';

class PlatformLocation {
  historyGo(relativePosition) {
    throw new Error('Not implemented');
  }
  static \u0275fac = i0.\u0275\u0275ngDeclareFactory({
    minVersion: "12.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: PlatformLocation,
    deps: [],
    target: i0.\u0275\u0275FactoryTarget.Injectable
  });
  static \u0275prov = i0.\u0275\u0275ngDeclareInjectable({
    minVersion: "12.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: PlatformLocation,
    providedIn: "platform",
    useClass: undefined
  });
}

export { PlatformLocation };
`

const NG_MODULE_CHUNK = `
import * as i0 from '@angular/core';

class CommonModule {
  static \u0275fac = i0.\u0275\u0275ngDeclareFactory({
    minVersion: "12.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: CommonModule,
    deps: [],
    target: i0.\u0275\u0275FactoryTarget.NgModule
  });
  static \u0275mod = i0.\u0275\u0275ngDeclareNgModule({
    minVersion: "14.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: CommonModule,
    imports: [],
    exports: []
  });
  static \u0275inj = i0.\u0275\u0275ngDeclareInjector({
    minVersion: "12.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: CommonModule
  });
}

export { CommonModule };
`

const PIPE_CHUNK = `
import * as i0 from '@angular/core';

class AsyncPipe {
  constructor(ref) {
    this._ref = ref;
  }
  static \u0275fac = i0.\u0275\u0275ngDeclareFactory({
    minVersion: "12.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: AsyncPipe,
    deps: [{ token: i0.ChangeDetectorRef }],
    target: i0.\u0275\u0275FactoryTarget.Pipe
  });
  static \u0275pipe = i0.\u0275\u0275ngDeclarePipe({
    minVersion: "14.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: AsyncPipe,
    isStandalone: false,
    name: "async",
    pure: false
  });
}

export { AsyncPipe };
`

describe('Angular linker - chunk file linking', () => {
  it('should link \u0275\u0275ngDeclareFactory and \u0275\u0275ngDeclareInjectable', () => {
    const result = linkAngularPackageSync(
      INJECTABLE_CHUNK,
      'node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs',
    )

    expect(result.linked).toBe(true)
    expect(result.code).not.toContain('\u0275\u0275ngDeclare')
  })

  it('should link \u0275\u0275ngDeclareNgModule and \u0275\u0275ngDeclareInjector', () => {
    const result = linkAngularPackageSync(
      NG_MODULE_CHUNK,
      'node_modules/@angular/common/fesm2022/_common_module-chunk.mjs',
    )

    expect(result.linked).toBe(true)
    expect(result.code).not.toContain('\u0275\u0275ngDeclare')
  })

  it('should link \u0275\u0275ngDeclarePipe', () => {
    const result = linkAngularPackageSync(
      PIPE_CHUNK,
      'node_modules/@angular/common/fesm2022/_pipes-chunk.mjs',
    )

    expect(result.linked).toBe(true)
    expect(result.code).not.toContain('\u0275\u0275ngDeclare')
  })

  it('should link through the Vite plugin when \u0275 is written as an escape', async () => {
    // esbuild's default `--charset=ascii` minify writes `ɵ` in identifiers as `\u0275`.
    const code = PIPE_CHUNK.replaceAll('\u0275', '\\u0275')
    const plugin = angularLinkerPlugin() as any
    const result = await plugin.transform.handler.call(
      {},
      code,
      'node_modules/@angular/common/fesm2022/_pipes-chunk.mjs',
    )

    expect(result?.code).toContain('definePipe')
    expect(result.code).not.toContain('ngDeclare')
  })

  it('should return linked: false for files without declarations', () => {
    const code = `
      export function helper() { return 42; }
    `
    const result = linkAngularPackageSync(
      code,
      'node_modules/@angular/common/fesm2022/_utils-chunk.mjs',
    )

    expect(result.linked).toBe(false)
  })
})

describe('Linker transform filter matching', () => {
  // These mirror the two-stage filter from angular-linker-plugin.ts:
  // 1. Broad static filter (NODE_MODULES_JS_REGEX) for Vite's filter mechanism
  // 2. Precise handler-level check (JS_EXT_REGEX) inside the transform handler
  const NODE_MODULES_JS_REGEX = /node_modules/
  const JS_EXT_REGEX = /\.[cm]?js(?:\?.*)?$/

  function matches(id: string) {
    return NODE_MODULES_JS_REGEX.test(id) && JS_EXT_REGEX.test(id)
  }

  it('should match standard Angular FESM files', () => {
    expect(matches('node_modules/@angular/common/fesm2022/common.mjs')).toBe(true)
  })

  it('should match chunk files', () => {
    expect(matches('node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs')).toBe(true)
  })

  it('should match absolute paths', () => {
    expect(
      matches(
        '/Users/dev/project/node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs',
      ),
    ).toBe(true)
  })

  it('should match paths with Vite query strings', () => {
    expect(matches('node_modules/@angular/common/fesm2022/common.mjs?v=abc123')).toBe(true)
  })

  it('should match chunk files with Vite query strings', () => {
    expect(
      matches('node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs?v=df7b0864'),
    ).toBe(true)
  })

  it('should match Windows-style backslash paths', () => {
    expect(matches('node_modules\\@angular\\common\\fesm2022\\common.mjs')).toBe(true)
  })

  it('should match .js and .cjs files', () => {
    expect(matches('node_modules/@ngrx/store/fesm2022/ngrx-store.js')).toBe(true)
    expect(matches('node_modules/some-lib/index.cjs')).toBe(true)
  })

  it('should match PrimeNG files (excluded from optimizeDeps)', () => {
    expect(matches('node_modules/primeng/fesm2022/primeng-table.mjs')).toBe(true)
    expect(matches('node_modules/primeng/fesm2022/primeng-table.mjs?v=abc123')).toBe(true)
  })

  it('should not match non-JS files', () => {
    expect(matches('node_modules/@angular/common/fesm2022/common.d.ts')).toBe(false)
  })

  it('should not match application source files', () => {
    expect(matches('src/app/app.component.ts')).toBe(false)
  })
})

/**
 * Angular's own FESM bundles reach core through a namespace import, so every
 * forwardRef they ship is `i0.forwardRef(...)` rather than a bare call. The TS
 * linker matches on `callee.getSymbolName()`, which returns the property name
 * for a member expression, so both forms must unwrap.
 *
 * Verbatim shape from `@angular/forms/fesm2022/signals.mjs`. Leaving the wrapper
 * in place emits `forwardRef(() => X).ɵfac(t)`; `forwardRef` returns the arrow
 * function it was handed, so `.ɵfac` is undefined and the provider throws the
 * first time it is instantiated — which is every `[formField]` render.
 */
const NAMESPACED_FORWARD_REF_INJECTABLE = `
import * as i0 from '@angular/core';

class InputValidityMonitor {
  static ɵfac = i0.ɵɵngDeclareFactory({
    minVersion: "12.0.0",
    version: "22.0.7",
    ngImport: i0,
    type: InputValidityMonitor,
    deps: [],
    target: i0.ɵɵFactoryTarget.Injectable
  });
  static ɵprov = i0.ɵɵngDeclareInjectable({
    minVersion: "12.0.0",
    version: "22.0.7",
    ngImport: i0,
    type: InputValidityMonitor,
    providedIn: 'root',
    useClass: i0.forwardRef(() => AnimationInputValidityMonitor)
  });
}
class AnimationInputValidityMonitor extends InputValidityMonitor {}
export { InputValidityMonitor };
`

describe('linker: namespaced forwardRef', () => {
  it('unwraps `i0.forwardRef()` in useClass so the factory delegates to a real class', () => {
    const result = linkAngularPackageSync(
      NAMESPACED_FORWARD_REF_INJECTABLE,
      '/node_modules/@angular/forms/fesm2022/signals.mjs',
    )

    expect(result.linked).toBe(true)
    expect(result.code).toContain('AnimationInputValidityMonitor.ɵfac(__ngFactoryType__)')
    // `forwardRef(...).ɵfac` is always a TypeError — the wrapper returns the arrow function.
    expect(result.code).not.toMatch(/forwardRef\(\(\)\s*=>\s*\w+\)\.ɵfac/)
  })
})

/**
 * Angular 22.2.2 dropped `img|src` / `video|src` from the URL security context and added a
 * runtime shim to compensate. Older runtimes still need `ɵɵsanitizeUrl` on those bindings to
 * unwrap `SafeUrl`s, so the linker must follow the application's `angularVersion`.
 */
const IMG_SRC_COMPONENT = `
import * as i0 from '@angular/core';

class Preview {
  static ɵcmp = i0.ɵɵngDeclareComponent({
    minVersion: "14.0.0",
    version: "21.0.0",
    ngImport: i0,
    type: Preview,
    isStandalone: true,
    selector: "x-preview",
    template: '<img [src]="url">'
  });
}
export { Preview };
`

describe('linker: angularVersion', () => {
  const id = 'node_modules/x-preview/fesm2022/x-preview.mjs'

  it('sanitizes img src when linking for Angular before 22.2.2', () => {
    const result = linkAngularPackageSync(IMG_SRC_COMPONENT, id, {
      angularVersion: { major: 22, minor: 2, patch: 1 },
    })

    expect(result.code).toContain('ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrl)')
  })

  it('assumes the latest Angular when no version is given', () => {
    expect(linkAngularPackageSync(IMG_SRC_COMPONENT, id).code).toContain(
      'ɵɵdomProperty("src",ctx.url)',
    )
  })

  it('forwards the plugin angularVersion through the Vite transform hook', async () => {
    const plugin = angularLinkerPlugin({
      angularVersion: { major: 22, minor: 2, patch: 1 },
    }) as any
    const result = await plugin.transform.handler.call({}, IMG_SRC_COMPONENT, id)

    expect(result?.code).toContain('ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrl)')
  })

  it('forwards the plugin angularVersion through the dependency optimizer', async () => {
    const plugin = angularLinkerPlugin({
      angularVersion: { major: 22, minor: 2, patch: 1 },
    }) as any
    plugin.configResolved({ optimizeDeps: {} })
    const [optimizerPlugin] = plugin.config({}, { command: 'serve' }).optimizeDeps.rolldownOptions
      .plugins
    const code = await optimizerPlugin.load.handler.call(
      { fs: { readFile: async () => IMG_SRC_COMPONENT } },
      id,
    )

    expect(code).toContain('ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrl)')
  })

  it('keys the dependency optimizer cache on angularVersion', () => {
    // Vite hashes optimizer plugin names, not their options, into the deps cache key.
    const optimizerPluginName = (plugin: any) =>
      plugin.config({}, { command: 'serve' }).optimizeDeps.rolldownOptions.plugins[0].name
    const nameFor = (angularVersion?: AngularVersion) =>
      optimizerPluginName(angularLinkerPlugin({ angularVersion }))

    expect(nameFor()).toBe('angular-linker')
    expect(nameFor({ major: 22, minor: 2, patch: 1 })).toBe('angular-linker@22.2.1')
    expect(nameFor({ major: 22, minor: 2, patch: 2 })).toBe('angular-linker@22.2.2')
    expect(nameFor({ major: 23, minor: 0, patch: 0, prerelease: 'next.1' })).toBe(
      'angular-linker@23.0.0-next.1',
    )
    // An empty prerelease is not a stable release for the linker, so it needs its own key.
    expect(nameFor({ major: 22, minor: 3, patch: 0, prerelease: '' })).toBe(
      'angular-linker@22.3.0-',
    )
  })

  it('forwards angular() angularVersion to the linker plugin', async () => {
    const link = async (angularVersion: AngularVersion) => {
      const plugin = angular({ angularVersion }).find(
        (candidate) => candidate.name === '@oxc-angular/vite-linker',
      ) as any
      return (await plugin.transform.handler.call({}, IMG_SRC_COMPONENT, id))?.code
    }

    expect(await link({ major: 22, minor: 2, patch: 1 })).toContain(
      'ɵɵdomProperty("src",ctx.url,i0.ɵɵsanitizeUrl)',
    )
    expect(await link({ major: 22, minor: 2, patch: 2 })).toContain('ɵɵdomProperty("src",ctx.url)')
  })
})

describe('linker: sourcemap', () => {
  const id = 'node_modules/@angular/common/fesm2022/_pipes-chunk.mjs'

  it('returns a source map only when requested', () => {
    expect(linkAngularPackageSync(PIPE_CHUNK, id).map ?? null).toBeNull()

    const result = linkAngularPackageSync(PIPE_CHUNK, id, { sourcemap: true })
    expect(result.linked).toBe(true)
    const map = JSON.parse(result.map!)
    expect(map.sources).toEqual([id])
    expect(map.mappings).not.toBe('')
  })

  it('forwards the plugin sourcemap option through the Vite transform hook', async () => {
    const transform = async (sourcemap?: boolean) =>
      (angularLinkerPlugin({ sourcemap }) as any).transform.handler.call({}, PIPE_CHUNK, id)

    expect((await transform()).map).toBeNull()
    expect(JSON.parse((await transform(true)).map).sources).toEqual([id])
  })

  it('returns the map from the dependency optimizer load hook', async () => {
    const plugin = angularLinkerPlugin({ sourcemap: true }) as any
    plugin.configResolved({ optimizeDeps: {} })
    const [optimizerPlugin] = plugin.config({}, { command: 'serve' }).optimizeDeps.rolldownOptions
      .plugins
    const result = await optimizerPlugin.load.handler.call(
      { fs: { readFile: async () => PIPE_CHUNK } },
      id,
    )

    expect(result.code).toContain('definePipe')
    expect(JSON.parse(result.map).sources).toEqual([id])
  })

  it('keys the dependency optimizer cache on sourcemap', () => {
    // The load hook returns `{ code, map }` instead of a string, so prebundled deps
    // must be rebuilt when the option changes.
    const nameFor = (options: Parameters<typeof angularLinkerPlugin>[0]) =>
      (angularLinkerPlugin(options) as any).config({}, { command: 'serve' }).optimizeDeps
        .rolldownOptions.plugins[0].name

    expect(nameFor({ sourcemap: false })).toBe('angular-linker')
    expect(nameFor({ sourcemap: true })).toBe('angular-linker+map')
    expect(nameFor({ angularVersion: { major: 22, minor: 2, patch: 2 }, sourcemap: true })).toBe(
      'angular-linker@22.2.2+map',
    )
  })

  it('follows angular() sourceMap.vendor, like Angular CLI', async () => {
    const link = async (sourceMap: PluginOptions['sourceMap']) => {
      const plugin = angular({ sourceMap }).find(
        (candidate) => candidate.name === '@oxc-angular/vite-linker',
      ) as any
      return plugin.transform.handler.call({}, PIPE_CHUNK, id)
    }

    // Vendor maps are off by default and `sourceMap: true` only covers scripts.
    expect((await link(undefined)).map).toBeNull()
    expect((await link(true)).map).toBeNull()
    expect((await link({ scripts: true })).map).toBeNull()
    expect((await link({ scripts: true, vendor: false })).map).toBeNull()
    expect((await link({ scripts: false, vendor: true })).map).toBeNull()
    expect(JSON.parse((await link({ scripts: true, vendor: true })).map).sources).toEqual([id])
    expect(JSON.parse((await link({ vendor: true })).map).sources).toEqual([id])
  })
})

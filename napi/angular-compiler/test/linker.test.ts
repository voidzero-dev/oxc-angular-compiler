import { describe, it, expect } from 'vitest'

import { linkAngularPackageSync } from '../index.js'

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

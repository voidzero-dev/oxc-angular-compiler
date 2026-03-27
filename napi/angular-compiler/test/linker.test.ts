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

describe('NODE_MODULES_JS_REGEX filter matching', () => {
  // This is the fixed regex from angular-linker-plugin.ts
  const NODE_MODULES_JS_REGEX = /node_modules[\\/].*\.[cm]?js(?:\?.*)?$/

  it('should match standard Angular FESM files', () => {
    expect(NODE_MODULES_JS_REGEX.test('node_modules/@angular/common/fesm2022/common.mjs')).toBe(
      true,
    )
  })

  it('should match chunk files', () => {
    expect(
      NODE_MODULES_JS_REGEX.test(
        'node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs',
      ),
    ).toBe(true)
  })

  it('should match absolute paths', () => {
    expect(
      NODE_MODULES_JS_REGEX.test(
        '/Users/dev/project/node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs',
      ),
    ).toBe(true)
  })

  it('should match paths with Vite query strings', () => {
    expect(
      NODE_MODULES_JS_REGEX.test('node_modules/@angular/common/fesm2022/common.mjs?v=abc123'),
    ).toBe(true)
  })

  it('should match chunk files with Vite query strings', () => {
    expect(
      NODE_MODULES_JS_REGEX.test(
        'node_modules/@angular/common/fesm2022/_platform_location-chunk.mjs?v=df7b0864',
      ),
    ).toBe(true)
  })

  it('should match Windows-style backslash paths', () => {
    expect(NODE_MODULES_JS_REGEX.test('node_modules\\@angular\\common\\fesm2022\\common.mjs')).toBe(
      true,
    )
  })

  it('should match .js and .cjs files', () => {
    expect(NODE_MODULES_JS_REGEX.test('node_modules/@ngrx/store/fesm2022/ngrx-store.js')).toBe(true)
    expect(NODE_MODULES_JS_REGEX.test('node_modules/some-lib/index.cjs')).toBe(true)
  })

  it('should not match non-JS files', () => {
    expect(NODE_MODULES_JS_REGEX.test('node_modules/@angular/common/fesm2022/common.d.ts')).toBe(
      false,
    )
    expect(NODE_MODULES_JS_REGEX.test('src/app/app.component.ts')).toBe(false)
  })
})

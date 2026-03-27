import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { describe, it, expect } from 'vitest'

import { linkAngularPackageSync } from '../index.js'

/**
 * Resolve the path to an Angular 21 package file.
 * The e2e app has Angular 21.2.2 installed with chunk files.
 */
function resolveAngular21File(subpath: string): string {
  return resolve(__dirname, '../e2e/app/node_modules/@angular', subpath)
}

describe('Angular linker - chunk file support', () => {
  it('should link ɵɵngDeclare calls in _platform_location-chunk.mjs', () => {
    const filepath = resolveAngular21File('common/fesm2022/_platform_location-chunk.mjs')
    const code = readFileSync(filepath, 'utf8')

    // Verify the chunk file has unlinked declarations
    expect(code).toContain('\u0275\u0275ngDeclare')

    const result = linkAngularPackageSync(code, filepath)

    expect(result.linked).toBe(true)
    expect(result.code).not.toContain('\u0275\u0275ngDeclare')
  })

  it('should link ɵɵngDeclare calls in _location-chunk.mjs', () => {
    const filepath = resolveAngular21File('common/fesm2022/_location-chunk.mjs')
    const code = readFileSync(filepath, 'utf8')

    // Verify the chunk file has unlinked declarations
    expect(code).toContain('\u0275\u0275ngDeclare')

    const result = linkAngularPackageSync(code, filepath)

    expect(result.linked).toBe(true)
    expect(result.code).not.toContain('\u0275\u0275ngDeclare')
  })

  it('should link ɵɵngDeclare calls in _common_module-chunk.mjs', () => {
    const filepath = resolveAngular21File('common/fesm2022/_common_module-chunk.mjs')
    const code = readFileSync(filepath, 'utf8')

    // Verify the chunk file has unlinked declarations
    expect(code).toContain('\u0275\u0275ngDeclare')

    const result = linkAngularPackageSync(code, filepath)

    expect(result.linked).toBe(true)
    expect(result.code).not.toContain('\u0275\u0275ngDeclare')
  })

  it('should link all Angular 21 chunk files in @angular/common', () => {
    const { readdirSync } = require('node:fs')
    const dir = resolveAngular21File('common/fesm2022')
    const files = readdirSync(dir) as string[]
    const chunkFiles = files.filter(
      (f: string) => f.startsWith('_') && f.endsWith('-chunk.mjs'),
    )

    expect(chunkFiles.length).toBeGreaterThan(0)

    for (const chunkFile of chunkFiles) {
      const filepath = resolve(dir, chunkFile)
      const code = readFileSync(filepath, 'utf8')

      if (!code.includes('\u0275\u0275ngDeclare')) {
        continue // Skip chunks without declarations
      }

      const result = linkAngularPackageSync(code, filepath)

      expect(result.linked, `${chunkFile} should be linked`).toBe(true)
      expect(
        result.code.includes('\u0275\u0275ngDeclare'),
        `${chunkFile} should not contain unlinked declarations`,
      ).toBe(false)
    }
  })
})

describe('NODE_MODULES_JS_REGEX filter matching', () => {
  // This is the fixed regex from angular-linker-plugin.ts
  const NODE_MODULES_JS_REGEX = /node_modules[\\/].*\.[cm]?js(?:\?.*)?$/

  it('should match standard Angular FESM files', () => {
    expect(
      NODE_MODULES_JS_REGEX.test('node_modules/@angular/common/fesm2022/common.mjs'),
    ).toBe(true)
  })

  it('should match Angular 21 chunk files', () => {
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
      NODE_MODULES_JS_REGEX.test(
        'node_modules/@angular/common/fesm2022/common.mjs?v=abc123',
      ),
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
    expect(
      NODE_MODULES_JS_REGEX.test(
        'node_modules\\@angular\\common\\fesm2022\\common.mjs',
      ),
    ).toBe(true)
  })

  it('should match .js and .cjs files', () => {
    expect(
      NODE_MODULES_JS_REGEX.test('node_modules/@ngrx/store/fesm2022/ngrx-store.js'),
    ).toBe(true)
    expect(
      NODE_MODULES_JS_REGEX.test('node_modules/some-lib/index.cjs'),
    ).toBe(true)
  })

  it('should not match non-JS files', () => {
    expect(
      NODE_MODULES_JS_REGEX.test('node_modules/@angular/common/fesm2022/common.d.ts'),
    ).toBe(false)
    expect(
      NODE_MODULES_JS_REGEX.test('src/app/app.component.ts'),
    ).toBe(false)
  })
})

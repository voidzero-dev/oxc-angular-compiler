import { execSync } from 'node:child_process'
import { existsSync, readdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { test, expect } from '@playwright/test'

const __dirname = fileURLToPath(new URL('.', import.meta.url))
const APP_DIR = join(__dirname, '../app')
const BUILD_OUT_DIR = join(APP_DIR, 'dist-minify')
const TEMP_CONFIG = join(APP_DIR, 'vite.config.minify.ts')

function writeBuildConfig(minify: boolean): void {
  writeFileSync(
    TEMP_CONFIG,
    `
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { angular } from '@oxc-angular/vite';
import { defineConfig } from 'vite';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);
const tsconfig = path.resolve(__dirname, './tsconfig.json');

export default defineConfig({
  plugins: [
    angular({
      tsconfig,
      liveReload: false,
      minifyComponentStyles: 'auto',
    }),
  ],
  build: {
    minify: ${minify},
    outDir: 'dist-minify',
    rollupOptions: {
      external: [/^@angular\\/.+$/, /^rxjs(?:\\/.+)?$/, /^tslib$/],
    },
  },
});
`.trim(),
    'utf-8',
  )
}

function cleanup(): void {
  rmSync(TEMP_CONFIG, { force: true })
  rmSync(BUILD_OUT_DIR, { recursive: true, force: true })
}

function readBuiltJs(): string {
  const assetDir = join(BUILD_OUT_DIR, 'assets')
  const files = existsSync(assetDir) ? readdirSync(assetDir) : []
  const jsFiles = files.filter((file) => file.endsWith('.js'))

  expect(jsFiles.length).toBeGreaterThan(0)

  return jsFiles.map((file) => readFileSync(join(assetDir, file), 'utf-8')).join('\n')
}

test.describe('build auto minify component styles', () => {
  test.afterEach(() => {
    cleanup()
  })

  test('minifies embedded component styles when build.minify is true', () => {
    writeBuildConfig(true)

    execSync('npx vite build --config vite.config.minify.ts', {
      cwd: APP_DIR,
      stdio: 'pipe',
      timeout: 60000,
    })

    const output = readBuiltJs()

    expect(output).toContain('.card-title[_ngcontent-%COMP%]{color:green;margin:0}')
  })

  test('keeps embedded component styles unminified when build.minify is false', () => {
    writeBuildConfig(false)

    execSync('npx vite build --config vite.config.minify.ts', {
      cwd: APP_DIR,
      stdio: 'pipe',
      timeout: 60000,
    })

    const output = readBuiltJs()

    expect(output).toContain(
      '.card-title[_ngcontent-%COMP%] {\\n  color: green;\\n  margin: 0;\\n}',
    )
  })
})

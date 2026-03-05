import { describe, it, expect } from 'vitest'
/**
 * Tests for SSR manifest generation.
 *
 * These tests verify that the Vite plugin correctly generates the Angular SSR
 * manifests required by AngularNodeAppEngine. Without these manifests, SSR fails with:
 *   "Angular app engine manifest is not set."
 *
 * See: https://github.com/voidzero-dev/oxc-angular-compiler/issues/60
 */

// Import the SSR manifest plugin directly
import {
  ssrManifestPlugin,
  generateAppManifestCode,
  generateAppEngineManifestCode,
} from '../vite-plugin/angular-ssr-manifest-plugin.js'

describe('SSR Manifest Generation (Issue #60)', () => {
  describe('generateAppManifestCode', () => {
    it('should generate valid app manifest code with bootstrap import', () => {
      const code = generateAppManifestCode({
        ssrEntryImport: './src/main.server',
        baseHref: '/',
        indexHtmlContent: '<html><body><app-root></app-root></body></html>',
      })

      expect(code).toContain('ɵsetAngularAppManifest')
      expect(code).toContain('./src/main.server')
      expect(code).toContain('bootstrap')
      expect(code).toContain('inlineCriticalCss')
      expect(code).toContain('index.server.html')
      expect(code).toContain('<html><body><app-root></app-root></body></html>')
    })

    it('should escape template literal characters in HTML', () => {
      const code = generateAppManifestCode({
        ssrEntryImport: './src/main.server',
        baseHref: '/',
        indexHtmlContent: '<html><body>${unsafe}`backtick`\\backslash</body></html>',
      })

      // Template literal chars should be escaped
      expect(code).toContain('\\${unsafe}')
      expect(code).toContain('\\`backtick\\`')
      expect(code).toContain('\\\\backslash')
      // The dollar sign should be escaped to prevent template literal injection
      expect(code).not.toMatch(/[^\\]\$\{unsafe\}/)
    })

    it('should use custom baseHref', () => {
      const code = generateAppManifestCode({
        ssrEntryImport: './src/main.server',
        baseHref: '/my-app/',
        indexHtmlContent: '<html></html>',
      })

      expect(code).toContain("baseHref: '/my-app/'")
    })
  })

  describe('generateAppEngineManifestCode', () => {
    it('should generate valid app engine manifest code', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/',
      })

      expect(code).toContain('ɵsetAngularAppEngineManifest')
      expect(code).toContain("basePath: '/'")
      expect(code).toContain('supportedLocales')
      expect(code).toContain('entryPoints')
      expect(code).toContain('allowedHosts')
    })

    it('should strip trailing slash from basePath (except root)', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/my-app/',
      })

      expect(code).toContain("basePath: '/my-app'")
    })

    it('should keep root basePath as-is', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/',
      })

      expect(code).toContain("basePath: '/'")
    })

    it('should include ɵgetOrCreateAngularServerApp in entry points', () => {
      const code = generateAppEngineManifestCode({
        basePath: '/',
      })

      expect(code).toContain('ɵgetOrCreateAngularServerApp')
      expect(code).toContain('ɵdestroyAngularServerApp')
      expect(code).toContain('ɵextractRoutesAndCreateRouteTree')
    })
  })

  describe('ssrManifestPlugin', () => {
    it('should create a plugin with correct name', () => {
      const plugin = ssrManifestPlugin({})
      expect(plugin.name).toBe('@oxc-angular/vite-ssr-manifest')
    })

    it('should only apply to build mode', () => {
      const plugin = ssrManifestPlugin({})
      expect(plugin.apply).toBe('build')
    })
  })
})

import { test, expect } from '../fixtures/test-fixture.js'

test.describe('TypeScript Component Full Reload', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('modifying .ts component file triggers full page reload', async ({
    page,
    fileModifier,
    hmrDetector,
  }) => {
    // Set up detection
    await hmrDetector.setupEventListeners()
    const sentinelId = await hmrDetector.addSentinel()

    // Verify initial content (title signal is "E2E_TITLE")
    await expect(page.locator('h1')).toContainText('E2E_TITLE')

    // Modify TypeScript component - change signal value
    await fileModifier.modifyFile('app.component.ts', (content) => {
      return content.replace("title = signal('E2E_TITLE')", "title = signal('TS_CHANGED')")
    })

    // Wait for page reload
    await page.waitForEvent('load', { timeout: 15000 })
    await page.waitForLoadState('networkidle')

    // Sentinel should be gone (page reloaded = entire DOM replaced)
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(false)

    // Verify new content is displayed
    await expect(page.locator('h1')).toContainText('TS_CHANGED')
  })

  test('adding new property to component triggers full reload', async ({
    page,
    fileModifier,
    hmrDetector,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Add a new signal property
    await fileModifier.modifyFile('app.component.ts', (content) => {
      return content.replace(
        "protected readonly title = signal('E2E_TITLE')",
        `protected readonly title = signal('E2E_TITLE')
  protected readonly newProperty = signal('NEW_PROPERTY')`,
      )
    })

    // Wait for reload
    await page.waitForEvent('load', { timeout: 15000 })
    await page.waitForLoadState('networkidle')

    // Sentinel should be gone
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(false)
  })

  test('modifying component decorator triggers full reload', async ({
    page,
    fileModifier,
    hmrDetector,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Modify the decorator (change selector)
    await fileModifier.modifyFile('app.component.ts', (content) => {
      return content.replace("selector: 'app-root'", "selector: 'app-root-modified'")
    })

    // Wait for reload
    await page.waitForEvent('load', { timeout: 15000 })
    await page.waitForLoadState('networkidle')

    // Sentinel should be gone
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(false)
  })

  test('changing import statements triggers full reload', async ({
    page,
    fileModifier,
    hmrDetector,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Add a new import
    await fileModifier.modifyFile('app.component.ts', (content) => {
      return content.replace(
        "import { Component, signal } from '@angular/core'",
        "import { Component, signal, computed } from '@angular/core'",
      )
    })

    // Wait for reload
    await page.waitForEvent('load', { timeout: 15000 })
    await page.waitForLoadState('networkidle')

    // Sentinel should be gone
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(false)
  })
})

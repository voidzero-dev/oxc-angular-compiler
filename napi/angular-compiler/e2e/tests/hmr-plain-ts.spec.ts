import { test, expect } from '../fixtures/test-fixture.js'

/**
 * Plain (non-component) `.ts` modules — utilities, services, constants,
 * route configs — must trigger a full page reload when edited. Angular's
 * runtime HMR only refreshes template/style metadata on already-mounted
 * instances; module bindings captured by component constructors are not
 * re-pulled, so Vite's default propagation accepts via the importing
 * component's HMR boundary without re-rendering, leaving the DOM stale.
 *
 * Matches Angular CLI's official behavior, where any non-component .ts
 * change drops out of the HMR-eligible path and reloads the page.
 */
test.describe('Plain TS full reload', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('modifying a plain util .ts file triggers full page reload', async ({
    page,
    fileModifier,
    hmrDetector,
  }) => {
    await hmrDetector.setupEventListeners()
    const sentinelId = await hmrDetector.addSentinel()

    // Baseline value comes from util.ts and is bound into AppComponent's template.
    await expect(page.locator('[data-test-util]')).toContainText('UTIL_INITIAL')

    await fileModifier.modifyFile('util.ts', (content) =>
      content.replace('UTIL_INITIAL', 'UTIL_RELOADED'),
    )

    // A full reload destroys the sentinel.
    await page.waitForEvent('load', { timeout: 15000 })
    await page.waitForLoadState('networkidle')

    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(false)
    await expect(page.locator('[data-test-util]')).toContainText('UTIL_RELOADED')
  })
})

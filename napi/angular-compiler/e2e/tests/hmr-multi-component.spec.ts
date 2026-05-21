import { test, expect } from '../fixtures/test-fixture.js'

/**
 * Two `@Component` classes declared in the same `.ts` file. Each must:
 *   - render correctly on initial load,
 *   - receive its own per-component HMR update when its template or styles
 *     change (NO full reload), without disturbing the sibling component.
 *
 * Guards the per-component cache + dispatch wiring (componentsByFile,
 * filePath@ClassName-keyed inlineTemplateCache / inlineStylesCache,
 * pendingHmrUpdates per componentId).
 */
test.describe('Multi-component file HMR', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('both components render on initial load', async ({ page }) => {
    await expect(page.locator('app-duo-first h3')).toContainText('DUO_FIRST_TITLE')
    await expect(page.locator('app-duo-second h3')).toContainText('DUO_SECOND_TITLE')
  })

  test('inline template change in the FIRST component triggers HMR (no reload)', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()
    await expect(page.locator('app-duo-first h3')).toContainText('DUO_FIRST_TITLE')

    await fileModifier.modifyFile('duo.component.ts', (content) =>
      content.replace('DUO_FIRST_TITLE', 'DUO_FIRST_HMR'),
    )
    await waitForHmr()

    await expect(page.locator('app-duo-first h3')).toContainText('DUO_FIRST_HMR')
    // Sibling untouched, no full reload.
    await expect(page.locator('app-duo-second h3')).toContainText('DUO_SECOND_TITLE')
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })

  test('inline template change in the SECOND component triggers HMR (no reload)', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()
    await expect(page.locator('app-duo-second h3')).toContainText('DUO_SECOND_TITLE')

    await fileModifier.modifyFile('duo.component.ts', (content) =>
      content.replace('DUO_SECOND_TITLE', 'DUO_SECOND_HMR'),
    )
    await waitForHmr()

    await expect(page.locator('app-duo-second h3')).toContainText('DUO_SECOND_HMR')
    await expect(page.locator('app-duo-first h3')).toContainText('DUO_FIRST_TITLE')
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })

  test('inline styles change in the SECOND component triggers HMR (no reload)', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()
    const before = await page
      .locator('app-duo-second .duo.second')
      .evaluate((el) => getComputedStyle(el).color)
    expect(before).toMatch(/rgb\(/)

    await fileModifier.modifyFile('duo.component.ts', (content) =>
      content.replace('--duo-second-color: steelblue', '--duo-second-color: green'),
    )
    await waitForHmr()

    // Color should have changed for the SECOND component; sentinel proves no reload.
    const after = await page
      .locator('app-duo-second .duo.second')
      .evaluate((el) => getComputedStyle(el).color)
    expect(after).not.toBe(before)
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })

  test('a non-template/styles edit in a multi-component file triggers full reload', async ({
    page,
    fileModifier,
    hmrDetector,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()
    await fileModifier.modifyFile('duo.component.ts', (content) =>
      content.replace(
        "first-component-in-multi-component-file'",
        "first-component-in-multi-component-file-MODIFIED'",
      ),
    )
    await page.waitForEvent('load', { timeout: 15000 })
    await page.waitForLoadState('networkidle')
    // Sentinel destroyed by full reload.
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(false)
    await expect(page.locator('app-duo-first p')).toContainText(
      'first-component-in-multi-component-file-MODIFIED',
    )
  })
})

import { test, expect } from '../fixtures/test-fixture.js'

/**
 * Inline style change in a `.ts` file ( `styles: ['…']` ) should trigger
 * an `angular:component-update` HMR event with no full reload — matches
 * Angular CLI's official behavior (which sends inline-style updates with
 * the slightly misleading `type: 'template'` discriminator).
 */
test.describe('Inline styles HMR', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('inline-style-only change in .ts triggers HMR (no reload)', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Baseline color from the fixture (--inline-card-color default = #444).
    const body = page.locator('app-inline-card .inline-card-body')
    await expect(body).toBeVisible()

    await fileModifier.modifyFile('inline-card.component.ts', (content) =>
      // Replace the fallback color in the inline style.
      content.replace('var(--inline-card-color, #444)', 'rgb(255, 0, 128)'),
    )
    await waitForHmr()

    const color = await body.evaluate((el) => getComputedStyle(el).color)
    expect(color).toBe('rgb(255, 0, 128)')

    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })
})

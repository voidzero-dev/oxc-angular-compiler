import { test, expect } from '../fixtures/test-fixture.js'

/**
 * Inline template change in a `.ts` file ( `template: \`…\`` ) should
 * trigger an `angular:component-update` HMR event with no full reload —
 * matches Angular CLI's official behavior.
 */
test.describe('Inline template HMR', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('inline-template-only change in .ts triggers HMR (no reload)', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()
    await expect(page.locator('app-inline-card h2')).toContainText('INLINE_TITLE')

    await fileModifier.modifyFile('inline-card.component.ts', (content) =>
      content.replace('INLINE_TITLE', 'INLINE_TEMPLATE_HMR'),
    )
    await waitForHmr()

    await expect(page.locator('app-inline-card h2')).toContainText('INLINE_TEMPLATE_HMR')
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })
})

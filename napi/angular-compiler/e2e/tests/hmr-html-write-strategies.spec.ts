import { test, expect, type WriteStrategy } from '../fixtures/test-fixture.js'

/**
 * Regression coverage for write-strategy regressions on the chokidar-based
 * watcher. Vite's chokidar (recursive fs.watch on the root) handles all of
 * these reliably; this matrix is the empirical guard against future
 * regressions on either the watcher backend or the handleHotUpdate dispatcher.
 */

const STRATEGIES: WriteStrategy[] = ['writeFile-in-place', 'writeFile-with-fsync', 'atomic-rename']

test.describe('HTML template HMR — write-strategy matrix', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  for (const strategy of STRATEGIES) {
    test(`triggers HMR via "${strategy}"`, async ({
      page,
      fileModifier,
      hmrDetector,
      waitForHmr,
    }) => {
      const sentinelId = await hmrDetector.addSentinel()
      await expect(page.locator('h1')).toContainText('E2E_TITLE')

      await fileModifier.modifyFile(
        'app.html',
        (content) => content.replace('{{ title() }}', `STRATEGY_${strategy.toUpperCase()}`),
        strategy,
      )
      await waitForHmr()

      await expect(page.locator('h1')).toContainText(`STRATEGY_${strategy.toUpperCase()}`)
      // Sentinel survives → HMR happened, no full reload.
      expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
    })
  }
})

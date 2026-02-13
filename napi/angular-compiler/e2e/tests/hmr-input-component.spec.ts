import { test, expect } from '../fixtures/test-fixture.js'

test.describe('Input Component HMR', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('component with inputs renders correctly on initial load', async ({ page }) => {
    // Verify the Card component renders with bound input values
    await expect(page.locator('.card-title')).toContainText('INPUT_TITLE')
    await expect(page.locator('.card-value')).toContainText('42')
  })

  test('CSS HMR on component with inputs preserves input bindings', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    // Verify initial state
    await expect(page.locator('.card-title')).toContainText('INPUT_TITLE')
    await expect(page.locator('.card-value')).toContainText('42')

    const sentinelId = await hmrDetector.addSentinel()

    // Change card styles - modify title color from green to red
    await fileModifier.modifyFile('card.css', (content) => {
      return content.replace('color: green', 'color: rgb(255, 0, 0)')
    })

    await waitForHmr()

    // Verify no full reload
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)

    // Verify style changed
    const titleColor = await page.locator('.card-title').evaluate((el) => {
      return getComputedStyle(el).color
    })
    expect(titleColor).toBe('rgb(255, 0, 0)')

    // Verify input bindings still work after HMR update
    // This exercises the inputConfig conditional in update_module.rs:
    // If inputs were corrupted, these values would be missing or wrong
    await expect(page.locator('.card-title')).toContainText('INPUT_TITLE')
    await expect(page.locator('.card-value')).toContainText('42')
  })

  test('template HMR on component with inputs preserves input bindings', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    // Verify initial state
    await expect(page.locator('.card-title')).toContainText('INPUT_TITLE')

    const sentinelId = await hmrDetector.addSentinel()

    // Modify the card template - add a prefix to the title, keeping the binding
    await fileModifier.modifyFile('card.html', (content) => {
      return content.replace('{{ cardTitle() }}', 'UPDATED: {{ cardTitle() }}')
    })

    await waitForHmr()

    // Verify no full reload
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)

    // Verify template changed AND input binding still works
    await expect(page.locator('.card-title')).toContainText('UPDATED: INPUT_TITLE')
    await expect(page.locator('.card-value')).toContainText('42')
  })

  test('multiple HMR updates on component with inputs do not corrupt inputs', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // First CSS change
    await fileModifier.modifyFile('card.css', (content) => {
      return content.replace('color: green', 'color: rgb(255, 0, 0)')
    })
    await waitForHmr()

    // Verify inputs survive first update
    await expect(page.locator('.card-title')).toContainText('INPUT_TITLE')
    await expect(page.locator('.card-value')).toContainText('42')

    // Second CSS change
    await fileModifier.modifyFile('card.css', (content) => {
      return content.replace('color: rgb(255, 0, 0)', 'color: rgb(0, 0, 255)')
    })
    await waitForHmr()

    // Verify inputs survive second update
    await expect(page.locator('.card-title')).toContainText('INPUT_TITLE')
    await expect(page.locator('.card-value')).toContainText('42')

    // Verify style reflects latest change
    const titleColor = await page.locator('.card-title').evaluate((el) => {
      return getComputedStyle(el).color
    })
    expect(titleColor).toBe('rgb(0, 0, 255)')

    // No reload through all updates
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })
})

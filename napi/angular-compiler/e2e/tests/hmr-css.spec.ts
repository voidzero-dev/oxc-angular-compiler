import { test, expect } from '../fixtures/test-fixture.js'

test.describe('CSS Style HMR', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('modifying .css file triggers HMR update without page reload', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    await hmrDetector.setupEventListeners()
    const sentinelId = await hmrDetector.addSentinel()

    // Capture initial computed style of h1
    const initialColor = await page.locator('h1').evaluate((el) => {
      return getComputedStyle(el).color
    })

    // Modify CSS to change h1 color to red
    await fileModifier.modifyFile('app.css', (content) => {
      return content + '\nh1 { color: rgb(255, 0, 0) !important; }\n'
    })

    await waitForHmr()

    // Verify no full reload
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)

    // Verify style changed
    const newColor = await page.locator('h1').evaluate((el) => {
      return getComputedStyle(el).color
    })
    expect(newColor).toBe('rgb(255, 0, 0)')
    expect(newColor).not.toBe(initialColor)
  })

  test('CSS variable changes apply via HMR without reload', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Modify CSS variable definition
    await fileModifier.modifyFile('app.css', (content) => {
      // Change primary-color variable from blue to red
      return content.replace('--primary-color: blue', '--primary-color: red')
    })

    await waitForHmr()

    // Verify no reload
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)

    // Verify the h1 color changed (it uses --primary-color)
    const newColor = await page.locator('h1').evaluate((el) => {
      return getComputedStyle(el).color
    })
    expect(newColor).toBe('rgb(255, 0, 0)')
  })

  test('multiple CSS changes trigger multiple HMR updates', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // First change - add red background to h1
    await fileModifier.modifyFile('app.css', (content) => {
      return content + '\nh1 { background-color: rgb(255, 0, 0) !important; }\n'
    })
    await waitForHmr()

    let bgColor = await page.locator('h1').evaluate((el) => {
      return getComputedStyle(el).backgroundColor
    })
    expect(bgColor).toBe('rgb(255, 0, 0)')

    // Second change - change to blue background
    await fileModifier.modifyFile('app.css', (content) => {
      return content.replace('background-color: rgb(255, 0, 0)', 'background-color: rgb(0, 0, 255)')
    })
    await waitForHmr()

    bgColor = await page.locator('h1').evaluate((el) => {
      return getComputedStyle(el).backgroundColor
    })
    expect(bgColor).toBe('rgb(0, 0, 255)')

    // Sentinel should still exist
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })

  test('adding new CSS class via HMR works correctly', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Add a new CSS class with distinct styling
    await fileModifier.modifyFile('app.css', (content) => {
      return (
        content +
        `
.hmr-test-class {
  border: 5px solid rgb(0, 255, 0) !important;
  padding: 20px !important;
}
`
      )
    })

    // Wait for CSS HMR to settle before modifying HTML
    await waitForHmr()

    // Now modify HTML to use the new class
    await fileModifier.modifyFile('app.html', (content) => {
      return content.replace('<h1>', '<h1 class="hmr-test-class">')
    })

    await waitForHmr()

    // Verify the new class is applied
    const border = await page.locator('h1').evaluate((el) => {
      return getComputedStyle(el).border
    })
    expect(border).toContain('rgb(0, 255, 0)')

    // No reload
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })
})

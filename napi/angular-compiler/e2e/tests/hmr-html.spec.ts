import { readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { test, expect } from '../fixtures/test-fixture.js'

const INDEX_HTML = join(fileURLToPath(new URL('.', import.meta.url)), '../app/index.html')

let indexHtmlBackup: string | undefined

async function modifyIndexHtml(modifier: (content: string) => string): Promise<void> {
  const content = await readFile(INDEX_HTML, 'utf-8')
  indexHtmlBackup ??= content
  await writeFile(INDEX_HTML, modifier(content))
}

async function restoreIndexHtml(): Promise<void> {
  if (indexHtmlBackup === undefined) return
  await writeFile(INDEX_HTML, indexHtmlBackup)
  indexHtmlBackup = undefined
}

test.describe('HTML Template HMR', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/')
    await page.waitForLoadState('networkidle')
  })

  test('modifying .html file triggers HMR update without page reload', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    // 1. Add sentinel to detect full reload
    const sentinelId = await hmrDetector.addSentinel()

    // 2. Verify initial content
    await expect(page.locator('h1')).toContainText('E2E_TITLE')

    // 3. Modify HTML template
    await fileModifier.modifyFile('app.html', (content) => {
      return content.replace('{{ title() }}', 'TEMPLATE CHANGED VIA HMR!')
    })

    // 4. Wait for HMR to propagate
    await waitForHmr()

    // 5. Verify DOM updated with new content
    await expect(page.locator('h1')).toContainText('TEMPLATE CHANGED VIA HMR!')

    // 6. Verify HMR occurred (not full reload) - sentinel should survive
    const sentinelExists = await hmrDetector.sentinelExists(sentinelId)
    expect(sentinelExists).toBe(true)
  })

  test('multiple template changes trigger multiple HMR updates without reload', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    await hmrDetector.setupEventListeners()
    const sentinelId = await hmrDetector.addSentinel()

    // First change
    await fileModifier.modifyFile('app.html', (content) =>
      content.replace('{{ title() }}', 'CHANGE 1'),
    )
    await waitForHmr()
    await expect(page.locator('h1')).toContainText('CHANGE 1')

    // Second change
    await fileModifier.modifyFile('app.html', (content) => content.replace('CHANGE 1', 'CHANGE 2'))
    await waitForHmr()
    await expect(page.locator('h1')).toContainText('CHANGE 2')

    // Sentinel should still exist (no reload occurred)
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })

  test('modifying text content via template HMR works correctly', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    const sentinelId = await hmrDetector.addSentinel()

    // Modify the paragraph text content
    await fileModifier.modifyFile('app.html', (content) => {
      return content.replace('E2E test fixture for HMR testing.', 'HMR UPDATED TEXT CONTENT!')
    })

    await waitForHmr()

    // Verify paragraph text was updated
    await expect(page.locator('p.description')).toContainText('HMR UPDATED TEXT CONTENT!')

    // Verify no reload - sentinel should survive
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
  })
})

test.describe('HTML Template HMR — no Vite full reload requested', () => {
  // Regression test for https://github.com/voidzero-dev/oxc-angular-compiler/issues/443
  //
  // The DOM-sentinel tests above only prove the browser did not act on a
  // reload. They pass even when the server puts a `full-reload` on the wire,
  // because Vite's client drops an `.html` payload whose path does not match
  // `location.pathname`. That guard is gone when the payload path is `*` —
  // which is what Vite sends in `middlewareMode`, and what the reporter of
  // #443 hit. So assert on the payload, not on its downstream effect.
  test('modifying .html file sends no full-reload payload', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    await hmrDetector.captureWirePayloads()
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await fileModifier.modifyFile('app.html', (content) =>
      content.replace('{{ title() }}', 'NO_RELOAD_PAYLOAD'),
    )
    await waitForHmr()

    await expect(page.locator('h1')).toContainText('NO_RELOAD_PAYLOAD')

    const payloads = await hmrDetector.getWirePayloads()
    expect(payloads.map((p) => p.event)).toContain('angular:component-update')
    expect(payloads.map((p) => p.type)).not.toContain('full-reload')
  })

  test('modifying index.html still sends a full-reload payload', async ({
    page,
    hmrDetector,
    waitForHmr,
  }) => {
    await hmrDetector.captureWirePayloads()
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    await modifyIndexHtml((content) => content.replace('</body>', '  <!-- edited -->\n  </body>'))
    try {
      await waitForHmr()
      const payloads = await hmrDetector.getWirePayloads()
      expect(payloads.filter((p) => p.type === 'full-reload').map((p) => p.path)).toContain(
        '/index.html',
      )
    } finally {
      await restoreIndexHtml()
    }
  })
})

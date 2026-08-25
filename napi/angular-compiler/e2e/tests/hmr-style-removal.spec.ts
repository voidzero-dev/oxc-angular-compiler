import { test, expect } from '../fixtures/test-fixture.js'

/**
 * Regression test for https://github.com/voidzero-dev/oxc-angular-compiler/issues/457
 *
 * Losing the LAST style is the one style change HMR used to get wrong. The
 * generated update module opens with `...ClassName.ɵcmp`, so every property it
 * does not emit keeps its previous value — and the `styles` property was
 * omitted for every "no styles" answer. The component updated, the old CSS
 * stayed applied, and only a full reload cleared it.
 *
 * The unit tests assert the module now carries an explicit `styles: []`. This
 * one asserts the thing a user actually sees: the CSS is gone from the page,
 * and the page did not reload to get there.
 */
test.describe('Removing a component last style', () => {
  test('emptying inline `styles` drops the CSS with no full reload', async ({
    page,
    fileModifier,
    hmrDetector,
    waitForHmr,
  }) => {
    // Must run before `goto` — Vite's client opens its socket during load,
    // and the wire is the only honest evidence about a reload request.
    await hmrDetector.captureWirePayloads()
    await page.goto('/')
    await page.waitForLoadState('networkidle')

    // The previous spec's teardown restores the file it edited, and that
    // write can still be in flight when this page connects — its `full-reload`
    // would otherwise land in this test's recording and in its DOM. Let the
    // stray settle first, then start measuring.
    await waitForHmr()
    const payloadsBeforeEdit = (await hmrDetector.getWirePayloads()).length
    const sentinelId = await hmrDetector.addSentinel()

    const card = page.locator('app-inline-card .inline-card')
    await expect(card).toBeVisible()

    // The fixture's inline styles put a dashed border on the card and make
    // the host a block. Both come from the component's own `styles`.
    const beforeBorder = await card.evaluate((el) => getComputedStyle(el).borderStyle)
    expect(beforeBorder).toBe('dashed')
    const beforeHostDisplay = await page
      .locator('app-inline-card')
      .evaluate((el) => getComputedStyle(el).display)
    expect(beforeHostDisplay).toBe('block')

    // Drop the last (and only) inline style. `styles: []` is the transition
    // the issue names: a component that HAD a style and now has none.
    await fileModifier.modifyFile('inline-card.component.ts', (content) => {
      const emptied = content.replace(/styles: \[[\s\S]*?\n {2}\],/, 'styles: [],')
      if (emptied === content) {
        throw new Error('failed to empty the inline `styles` array in the fixture')
      }
      return emptied
    })
    await waitForHmr()

    // The component is still mounted — this is an update, not a teardown.
    await expect(page.locator('app-inline-card h2')).toHaveText('INLINE_TITLE')

    // The CSS is gone: the border reverts to the UA default, and the host
    // stops being a block. Before the fix these kept their old values.
    const afterBorder = await card.evaluate((el) => getComputedStyle(el).borderStyle)
    expect(afterBorder).toBe('none')
    const afterHostDisplay = await page
      .locator('app-inline-card')
      .evaluate((el) => getComputedStyle(el).display)
    expect(afterHostDisplay).toBe('inline')

    // And no full reload got us there. The sentinel proves the browser did
    // not act on one; the wire proves the server never asked for one.
    expect(await hmrDetector.sentinelExists(sentinelId)).toBe(true)
    const payloads = (await hmrDetector.getWirePayloads()).slice(payloadsBeforeEdit)
    expect(payloads.map((p) => p.event)).toContain('angular:component-update')
    expect(payloads.map((p) => p.type)).not.toContain('full-reload')
  })
})

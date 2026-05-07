import { readFile, writeFile, rename, unlink, open } from 'node:fs/promises'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'

import { test as base, expect, type Page } from '@playwright/test'

const __dirname = fileURLToPath(new URL('.', import.meta.url))
const FIXTURE_APP = join(__dirname, '../app/src/app')

/**
 * Write strategies that mimic how different tools / editors save files.
 * Used to guard against watcher implementations that miss certain patterns
 * (e.g., a per-file `node:fs.watch` on macOS dropping single fast in-place
 * writes from AI-tool Edit operations).
 */
export type WriteStrategy =
  | 'writeFile-in-place' // single fs.writeFile (Claude Code, most CLI tools)
  | 'writeFile-with-fsync' // writeFile + explicit fsync
  | 'atomic-rename' // write `.tmp` then rename (vim, IntelliJ "safe write")
  | 'truncate-then-write' // writeFile('') + delay + writeFile(content)

async function performWrite(
  filepath: string,
  content: string,
  strategy: WriteStrategy,
): Promise<void> {
  switch (strategy) {
    case 'writeFile-in-place':
      await writeFile(filepath, content)
      return
    case 'writeFile-with-fsync': {
      await writeFile(filepath, content)
      const handle = await open(filepath, 'r+')
      try {
        await handle.sync()
      } finally {
        await handle.close()
      }
      return
    }
    case 'atomic-rename': {
      const tmp = `${filepath}.hmr-${Date.now()}.tmp`
      await writeFile(tmp, content)
      try {
        await rename(tmp, filepath)
      } catch (err) {
        await unlink(tmp).catch(() => {})
        throw err
      }
      return
    }
    case 'truncate-then-write':
      await writeFile(filepath, '')
      await new Promise((r) => setTimeout(r, 30))
      await writeFile(filepath, content)
      return
  }
}

/**
 * File modification utility for e2e tests.
 * Backs up files before modification and restores them after tests.
 */
export class FileModifier {
  private originalContents: Map<string, string> = new Map()

  /**
   * Modify a file in the fixture app directory.
   * Automatically backs up the original content for restoration.
   *
   * Optionally takes a write strategy to mimic how different tools save —
   * use this to assert HMR works regardless of the consumer's editor /
   * AI-tool save pattern.
   */
  async modifyFile(
    filename: string,
    modifier: (content: string) => string,
    strategy: WriteStrategy = 'writeFile-in-place',
  ): Promise<void> {
    const filepath = join(FIXTURE_APP, filename)
    const content = await readFile(filepath, 'utf-8')

    if (!this.originalContents.has(filename)) {
      this.originalContents.set(filename, content)
    }

    const modified = modifier(content)
    await performWrite(filepath, modified, strategy)
  }

  /**
   * Restore a specific file to its original content.
   */
  async restoreFile(filename: string): Promise<void> {
    const original = this.originalContents.get(filename)
    if (original) {
      const filepath = join(FIXTURE_APP, filename)
      await writeFile(filepath, original)
      this.originalContents.delete(filename)
    }
  }

  /**
   * Restore all modified files to their original content.
   */
  async restoreAll(): Promise<void> {
    for (const [filename] of this.originalContents) {
      await this.restoreFile(filename)
    }
  }
}

/**
 * HMR detection utility.
 * Uses DOM sentinel approach to reliably detect HMR vs full page reload.
 */
export class HmrDetector {
  constructor(private page: Page) {}

  /**
   * Add a DOM sentinel element that will survive HMR but be destroyed on full reload.
   * @returns The sentinel ID for later checking
   */
  async addSentinel(): Promise<string> {
    const sentinelId = `hmr-sentinel-${Date.now()}`
    await this.page.evaluate((id) => {
      const el = document.createElement('div')
      el.id = id
      el.style.display = 'none'
      document.body.appendChild(el)
    }, sentinelId)
    return sentinelId
  }

  /**
   * Check if a sentinel element still exists in the DOM.
   * - Exists: HMR occurred (DOM was mutated, not replaced)
   * - Gone: Full page reload (entire DOM was replaced)
   */
  async sentinelExists(sentinelId: string): Promise<boolean> {
    return (await this.page.locator(`#${sentinelId}`).count()) > 0
  }

  /**
   * Set up listeners to capture HMR events from the page.
   * Call this before making file changes.
   * Note: We use addScriptTag to inject the listener code because
   * page.evaluate() cannot serialize import.meta.hot references.
   */
  async setupEventListeners(): Promise<void> {
    await this.page.addScriptTag({
      content: `
        window.__hmrEvents = [];
        if (import.meta.hot) {
          import.meta.hot.on("angular:component-update", (data) => {
            window.__hmrEvents.push({
              type: "angular:component-update",
              data,
              timestamp: Date.now(),
            });
          });
          import.meta.hot.on("vite:beforeFullReload", () => {
            window.__hmrEvents.push({
              type: "vite:beforeFullReload",
              timestamp: Date.now(),
            });
          });
        }
      `,
      type: 'module',
    })
  }

  /**
   * Get all captured HMR events.
   */
  async getEvents(): Promise<Array<{ type: string; data?: any; timestamp: number }>> {
    return await this.page.evaluate(() => (window as any).__hmrEvents || [])
  }

  /**
   * Check if a specific event type was received.
   */
  async hasEvent(eventType: string): Promise<boolean> {
    const events = await this.getEvents()
    return events.some((e) => e.type === eventType)
  }
}

// Custom test fixtures
type HmrTestFixtures = {
  fileModifier: FileModifier
  hmrDetector: HmrDetector
  waitForHmr: () => Promise<void>
}

export const test = base.extend<HmrTestFixtures>({
  // File modification utility with automatic cleanup
  fileModifier: async ({ page: _ }, use) => {
    const modifier = new FileModifier()
    await use(modifier)
    // Always restore files after test
    await modifier.restoreAll()
  },

  // HMR detection utility
  hmrDetector: async ({ page }, use) => {
    const detector = new HmrDetector(page)
    await use(detector)
  },

  // Wait for HMR updates to stabilize
  waitForHmr: async ({ page }, use) => {
    const wait = async () => {
      // Give time for file watcher to detect change and HMR to propagate
      // Note: Don't use networkidle - it never completes when HMR is active
      await page.waitForTimeout(2000)
    }
    await use(wait)
  },
})

export { expect }

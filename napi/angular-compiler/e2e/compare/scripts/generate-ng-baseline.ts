/**
 * Compiles every fixture with ngtsc and saves the outputs, for `compare --fixtures --ng-baseline`.
 * It loads only the Angular side of the harness, never Oxc or the comparison.
 *
 * Usage: oxnode scripts/generate-ng-baseline.ts [--category <name>]... [--save-ng-baseline <path>]
 */
import { resolve } from 'node:path'
import { parseArgs } from 'node:util'

import { discoverFixtures } from '../fixtures/index.js'
import { compileReference, ngBaselineKey, ngtscCompilation } from '../fixtures/ng-baseline.js'
import { printDiagnosticWarningSummary } from '../src/compilers/angular-ngtsc.js'
import { saveNgBaseline } from '../src/ng-baseline.js'

const { values } = parseArgs({
  options: {
    category: { type: 'string', multiple: true },
    'save-ng-baseline': { type: 'string', default: 'ng-baseline.json' },
  },
})

const outputs: Record<string, { code: string; error?: string }> = {}
for (const fixture of await discoverFixtures(values.category)) {
  const compilation = ngtscCompilation(fixture)
  if (compilation) {
    const { code, error } = await compileReference(fixture).catch((error: unknown) => ({
      code: '',
      error: String(error),
    }))
    outputs[ngBaselineKey(compilation)] = { code, error }
  }
}
printDiagnosticWarningSummary()

// Sorted by key, without timings: the same fixtures and packages give the same file.
const sorted = Object.entries(outputs).sort(([a], [b]) => (a < b ? -1 : 1))
const path = resolve(values['save-ng-baseline'])
await saveNgBaseline(Object.fromEntries(sorted), path)
console.log(`ngtsc baseline saved to: ${path} (${sorted.length} outputs)`)

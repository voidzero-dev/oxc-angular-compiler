/**
 * Angular NgtscProgram baseline snapshot.
 *
 * Separates the slow Angular TSC compilation from the fast Oxc compilation + comparison.
 * Generate once with `--generate-ng-baseline`, reuse many times with `--ng-baseline <path>`.
 */

import { readFile, writeFile } from 'node:fs/promises'

import { VERSION as ANGULAR_VERSION } from '@angular/compiler'

/**
 * Angular baseline data - snapshot of NgtscProgram compilation output.
 */
export interface NgBaselineData {
  metadata: {
    generatedAt: string
    angularVersion: string
    tsconfigPath: string
    projectRoot: string
    totalFiles: number
    emittedFiles: number
    durationMs: number
  }
  /** Map of source file path -> emitted JS content (null if file failed to emit) */
  files: Record<string, string | null>
}

/**
 * Save Angular baseline data to a JSON file.
 */
export async function saveNgBaseline(data: NgBaselineData, outputPath: string): Promise<void> {
  // No pretty-printing: baseline files are ~60-70MB for large projects.
  // Compact JSON is faster to write/parse and ~10% smaller.
  await writeFile(outputPath, JSON.stringify(data), 'utf-8')
}

/**
 * Load Angular baseline data from a JSON file.
 */
export async function loadNgBaseline(baselinePath: string): Promise<NgBaselineData> {
  const content = await readFile(baselinePath, 'utf-8')
  return JSON.parse(content) as NgBaselineData
}

/**
 * Create baseline data from NgtscFileEmitter results.
 */
export function createNgBaselineData(
  emittedFiles: Map<string, string>,
  tsconfigPath: string,
  projectRoot: string,
  totalFiles: number,
  durationMs: number,
): NgBaselineData {
  const files: Record<string, string | null> = {}
  for (const [filePath, content] of emittedFiles) {
    files[filePath] = content
  }

  let angularVersion: string
  try {
    angularVersion = ANGULAR_VERSION.full
  } catch {
    angularVersion = 'unknown'
  }

  return {
    metadata: {
      generatedAt: new Date().toISOString(),
      angularVersion,
      tsconfigPath,
      projectRoot,
      totalFiles,
      emittedFiles: emittedFiles.size,
      durationMs,
    },
    files,
  }
}

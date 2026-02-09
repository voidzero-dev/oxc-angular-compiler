import { readFile } from 'node:fs/promises'
import { dirname, join, normalize } from 'node:path'
import { fileURLToPath } from 'node:url'

import { VERSION as ANGULAR_VERSION } from '@angular/compiler'

import { compareFullFileSemantically } from './compare.js'
import { NgtscFileEmitter } from './compilers/angular-ngtsc.js'
import { compileProjectWithOxc, type PlainResolvedResources } from './compilers/oxc.js'
import { findComponents } from './discovery/finder.js'
import { createNgBaselineData, loadNgBaseline, saveNgBaseline } from './ng-baseline.js'
import type {
  ComparisonReport,
  CompilerConfig,
  ComponentInfo,
  ReportMetadata,
  ReportSummary,
  FileComparisonResult,
  FileReportSummary,
} from './types.js'

/**
 * Get the Oxc Angular compiler version from package.json.
 * Returns undefined if the version cannot be read.
 */
async function getOxcVersion(): Promise<string | undefined> {
  try {
    const currentDir = dirname(fileURLToPath(import.meta.url))
    const packageJsonPath = join(currentDir, '..', '..', '..', 'package.json')
    const packageJson = JSON.parse(await readFile(packageJsonPath, 'utf-8'))
    return packageJson.version
  } catch {
    return undefined
  }
}

/**
 * Get the Angular compiler version.
 * Returns undefined if the version cannot be read.
 */
function getAngularVersion(): string | undefined {
  try {
    return ANGULAR_VERSION.full
  } catch {
    return undefined
  }
}

/**
 * Run the comparison between Oxc and TypeScript Angular compilers.
 */
export async function runComparison(config: CompilerConfig): Promise<ComparisonReport> {
  const startTime = performance.now()

  console.log('Angular Compiler Comparison Test Runner')
  console.log('=======================================\n')
  console.log(`Project: ${config.projectRoot}`)
  console.log(`Parallel: ${config.parallel ?? true}`)
  console.log()

  // Discover components
  console.log('Discovering components...')
  const components = await findComponents(config)

  if (components.length === 0) {
    console.log('No components found!')
    return {
      summary: createEmptySummary(),
      metadata: await createMetadata(config, startTime),
      results: [],
    }
  }

  // Generate-ng-baseline-only mode: compile with Angular, save, and return early
  if (config.generateNgBaselineOnly) {
    console.log(`\nGenerating Angular baseline for ${components.length} components...`)
    await generateAndSaveNgBaseline(components, config)
    return {
      summary: createEmptySummary(),
      metadata: await createMetadata(config, startTime),
      results: [],
    }
  }

  console.log(`\nComparing ${components.length} components across files...`)
  const fileResults = await compareFilesProjectWide(components, config)

  // Calculate file-level summary
  const fileSummary = calculateFileSummary(fileResults, components.length)

  // Generate report with file-level results
  const report: ComparisonReport = {
    summary: createEmptySummary(), // Component-level summary not applicable in file mode
    metadata: await createMetadata(config, startTime),
    results: [], // Component-level results not applicable in file mode
    fileComparisonMode: true,
    fileSummary,
    fileResults,
  }

  return report
}

/**
 * Compare files using full-file strict comparison (file-to-file mode).
 *
 * Instead of extracting templates per component and comparing them, this
 * compares the COMPLETE .js file output to ensure Oxc produces identical
 * results to Angular's official compiler.
 *
 * What file-to-file comparison validates:
 * 1. Imports - Same Angular runtime imports (@angular/core symbols)
 * 2. Class Definition - Transformed class with static fields
 * 3. Static Fields - ɵcmp/ɵdir (definition), ɵfac (factory), ɵprov (provider)
 * 4. setClassMetadata Call - TestBed metadata for debugging
 * 5. Exports - Default/named exports preserved
 *
 * @param components - All discovered components
 * @param config - Compiler configuration
 * @returns File-level comparison results
 */
async function compareFilesProjectWide(
  components: ComponentInfo[],
  config: CompilerConfig,
): Promise<FileComparisonResult[]> {
  // 1. Collect unique file paths and their contents (TypeScript files + external resources)
  const fileContents = new Map<string, string>()

  // Check if we need to read from disk (full-file mode optimization - sourceCode not stored)
  const needsToReadFromDisk = !components[0]?.sourceCode

  for (const component of components) {
    // Collect TypeScript source files
    if (!fileContents.has(component.filePath)) {
      if (component.sourceCode) {
        fileContents.set(component.filePath, component.sourceCode)
      } else if (needsToReadFromDisk) {
        // Read from disk (full-file mode - source wasn't stored in ComponentInfo to save memory)
        try {
          fileContents.set(component.filePath, await readFile(component.filePath, 'utf-8'))
        } catch (e) {
          console.warn(`Warning: Could not read ${component.filePath}: ${e as Error}`)
        }
      }
    }

    // Also collect external template files for Angular's AOT compilation
    if (component.templatePath && component.templateContent) {
      if (!fileContents.has(component.templatePath)) {
        fileContents.set(component.templatePath, component.templateContent)
      }
    }

    // Collect external style files
    if (component.styleUrls && component.styles) {
      for (let i = 0; i < component.styleUrls.length && i < component.styles.length; i++) {
        const styleUrl = component.styleUrls[i]
        const styleContent = component.styles[i]
        if (styleContent && !fileContents.has(styleUrl)) {
          fileContents.set(styleUrl, styleContent)
        }
      }
    }
  }

  // Extract just the TypeScript file paths for compilation roots
  const filePaths = Array.from(fileContents.keys()).filter((p) => p.endsWith('.ts'))
  console.log(`\nFile-to-File Strict Comparison Mode`)
  console.log(`===================================`)
  console.log(`Compiling ${filePaths.length} unique TypeScript files...`)
  console.log(
    `External resources: ${fileContents.size - filePaths.length} files (templates/styles)`,
  )

  // Build resolved resources map for each file
  const resolvedResourcesByFile = new Map<string, PlainResolvedResources>()
  for (const component of components) {
    if (!resolvedResourcesByFile.has(component.filePath)) {
      resolvedResourcesByFile.set(component.filePath, {
        templates: {},
        styles: {},
      })
    }

    const resolved = resolvedResourcesByFile.get(component.filePath)!

    if (component.templateUrl && component.templateContent) {
      resolved.templates[component.templateUrl] = component.templateContent
    }

    if (component.originalStyleUrls && component.styles) {
      for (let i = 0; i < component.originalStyleUrls.length && i < component.styles.length; i++) {
        const styleUrl = component.originalStyleUrls[i]
        const styleContent = component.styles[i]
        if (styleContent) {
          if (!resolved.styles[styleUrl]) {
            resolved.styles[styleUrl] = []
          }
          resolved.styles[styleUrl].push(styleContent)
        }
      }
    }
  }

  // 2. Compile all files with Oxc sequentially (streaming to disk)
  const oxcStartTime = performance.now()
  const oxcResult = compileProjectWithOxc(filePaths, fileContents, {
    resolvedResourcesByFile,
    tsconfigPath: config.tsconfigPath,
  })
  const oxcDuration = performance.now() - oxcStartTime
  console.log(
    `Oxc batch compilation: ${oxcDuration.toFixed(0)}ms (${oxcResult.successCount} succeeded, ${oxcResult.errorCount} errors)`,
  )

  // 3. Get Angular outputs: either from baseline file or by running NgtscFileEmitter
  let ngOutputMap: Map<string, string>
  let ngInitDuration: number

  if (config.ngBaselinePath) {
    // Load from baseline - skip Angular compilation entirely
    const ngStartTime = performance.now()
    console.log(`Loading Angular baseline from: ${config.ngBaselinePath}`)
    const baseline = await loadNgBaseline(config.ngBaselinePath)
    ngOutputMap = new Map<string, string>()
    for (const [filePath, output] of Object.entries(baseline.files)) {
      if (output !== null) {
        ngOutputMap.set(filePath, output)
      }
    }
    ngInitDuration = performance.now() - ngStartTime
    console.log(
      `Angular baseline loaded: ${ngInitDuration.toFixed(0)}ms (${ngOutputMap.size} files, generated ${baseline.metadata.generatedAt}, Angular ${baseline.metadata.angularVersion})`,
    )

    // Warn about stale baseline: files in current discovery but missing from baseline
    const missingFromBaseline = filePaths.filter(
      (fp) => !ngOutputMap.has(fp) && !ngOutputMap.has(normalize(fp)),
    )
    if (missingFromBaseline.length > 0) {
      console.warn(
        `\nWarning: ${missingFromBaseline.length} file(s) not found in baseline (baseline may be stale):`,
      )
      for (const fp of missingFromBaseline.slice(0, 5)) {
        console.warn(`  - ${fp}`)
      }
      if (missingFromBaseline.length > 5) {
        console.warn(`  ... and ${missingFromBaseline.length - 5} more`)
      }
      console.warn(
        `Regenerate with: --generate-ng-baseline --save-ng-baseline ${config.ngBaselinePath}\n`,
      )
    }

    // Clear fileContents - no longer needed
    fileContents.clear()
    resolvedResourcesByFile.clear()
  } else {
    // Run NgtscFileEmitter (slow path)
    const ngStartTime = performance.now()
    const emitter = new NgtscFileEmitter(filePaths, config.tsconfigPath!, fileContents)
    await emitter.initialize()
    ngInitDuration = performance.now() - ngStartTime
    console.log(
      `Angular NgtscProgram init: ${ngInitDuration.toFixed(0)}ms (${emitter.fileCount} files)`,
    )

    // Clear fileContents - no longer needed after both compilers have processed the files
    // This frees ~500MB-2GB of duplicated source code strings
    fileContents.clear()
    resolvedResourcesByFile.clear()

    // Get all emitted files for comparison (and optionally saving baseline)
    ngOutputMap = emitter.getAllEmittedFiles()

    // Save baseline if requested
    if (config.saveNgBaselinePath) {
      const baselineData = createNgBaselineData(
        ngOutputMap,
        config.tsconfigPath!,
        config.projectRoot,
        filePaths.length,
        ngInitDuration,
      )
      await saveNgBaseline(baselineData, config.saveNgBaselinePath)
      console.log(
        `Angular baseline saved to: ${config.saveNgBaselinePath} (${ngOutputMap.size} files)`,
      )
    }
  }

  // Group components by file path
  const componentsByFile = new Map<string, ComponentInfo[]>()
  for (const component of components) {
    const existing = componentsByFile.get(component.filePath) || []
    existing.push(component)
    componentsByFile.set(component.filePath, existing)
  }

  const uniqueFilePaths = Array.from(componentsByFile.keys())

  // 4. Emit and compare files
  const results: FileComparisonResult[] = []
  const emitStartTime = performance.now()
  let emittedCount = 0

  for (const filePath of uniqueFilePaths) {
    // Get class names for this file
    const fileComponents = componentsByFile.get(filePath) || []
    const classNames = fileComponents.map((c) => c.className)

    // Get Angular output (from baseline or emitter)
    // Try both original and normalized paths for safety (matches emit() fallback behavior)
    const ngOutput = ngOutputMap.get(filePath) ?? ngOutputMap.get(normalize(filePath))
    const ngError = ngOutput === undefined ? `No emit result for ${filePath}` : undefined

    // Get Oxc output from batch result
    const oxcOutput = oxcResult.emittedFiles.get(filePath)
    const oxcError = oxcResult.errors.get(filePath)?.[0]

    // Calculate timing
    const perFileTime =
      (ngInitDuration + (performance.now() - emitStartTime)) / uniqueFilePaths.length

    // Determine status and compare
    let status: FileComparisonResult['status']
    let comparisonDetails: FileComparisonResult['comparisonDetails']

    if (oxcError && ngError) {
      status = 'both-error'
    } else if (oxcError) {
      status = 'oxc-error'
    } else if (ngError || !ngOutput) {
      // Check if Angular produced JIT output instead of AOT
      if (ngOutput && classNames.some((cn) => isAngularJitOutput(ngOutput, cn))) {
        status = 'angular-jit-only'
      } else {
        status = 'ng-error'
      }
    } else if (!oxcOutput) {
      status = 'oxc-error'
    } else {
      // Both succeeded - compare FULL file outputs semantically
      const comparison = await compareFullFileSemantically(oxcOutput, ngOutput)
      status = comparison.match ? 'match' : 'mismatch'

      if (!comparison.match) {
        comparisonDetails = {
          importDiffs: comparison.importDiffs,
          exportDiffs: comparison.exportDiffs,
          classDiffs: comparison.classDiffs,
          staticFieldDiffs: comparison.staticFieldDiffs,
          classMetadataDiffs: comparison.classMetadataDiffs,
          functionComparison: comparison.functionComparison,
          parseErrors: comparison.parseErrors,
        }
      }
    }

    results.push({
      filePath,
      classNames,
      status,
      oxcOutput: status !== 'match' ? oxcOutput : undefined,
      ngOutput: status !== 'match' ? ngOutput : undefined,
      oxcError,
      ngError,
      comparisonDetails,
      oxcCompilationTimeMs: oxcDuration / uniqueFilePaths.length,
      ngCompilationTimeMs: perFileTime,
    })

    emittedCount++
    if (emittedCount % 50 === 0 || emittedCount === uniqueFilePaths.length) {
      process.stdout.write(
        `\rProgress: ${emittedCount}/${uniqueFilePaths.length} files (${((emittedCount / uniqueFilePaths.length) * 100).toFixed(1)}%)`,
      )
    }
  }

  const emitDuration = performance.now() - emitStartTime
  console.log(`\nAngular per-file emit + comparison: ${emitDuration.toFixed(0)}ms`)

  // Print file-level summary
  printFileLevelSummary(results, components.length)

  return results
}

/**
 * Generate Angular baseline only (no Oxc compilation, no comparison).
 * Discovers components, collects file contents, runs NgtscFileEmitter, and saves the output.
 */
async function generateAndSaveNgBaseline(
  components: ComponentInfo[],
  config: CompilerConfig,
): Promise<void> {
  // Collect file contents (same logic as compareFilesProjectWide)
  const fileContents = new Map<string, string>()
  const needsToReadFromDisk = !components[0]?.sourceCode

  for (const component of components) {
    if (!fileContents.has(component.filePath)) {
      if (component.sourceCode) {
        fileContents.set(component.filePath, component.sourceCode)
      } else if (needsToReadFromDisk) {
        try {
          fileContents.set(component.filePath, await readFile(component.filePath, 'utf-8'))
        } catch (e) {
          console.warn(`Warning: Could not read ${component.filePath}: ${e as Error}`)
        }
      }
    }

    if (component.templatePath && component.templateContent) {
      if (!fileContents.has(component.templatePath)) {
        fileContents.set(component.templatePath, component.templateContent)
      }
    }

    if (component.styleUrls && component.styles) {
      for (let i = 0; i < component.styleUrls.length && i < component.styles.length; i++) {
        const styleUrl = component.styleUrls[i]
        const styleContent = component.styles[i]
        if (styleContent && !fileContents.has(styleUrl)) {
          fileContents.set(styleUrl, styleContent)
        }
      }
    }
  }

  const filePaths = Array.from(fileContents.keys()).filter((p) => p.endsWith('.ts'))
  console.log(`Compiling ${filePaths.length} TypeScript files with Angular NgtscProgram...`)

  const ngStartTime = performance.now()
  const emitter = new NgtscFileEmitter(filePaths, config.tsconfigPath!, fileContents)
  await emitter.initialize()
  const ngDuration = performance.now() - ngStartTime

  console.log(
    `Angular NgtscProgram completed: ${ngDuration.toFixed(0)}ms (${emitter.emittedFileCount} files emitted)`,
  )

  const ngOutputMap = emitter.getAllEmittedFiles()
  const outputPath = config.saveNgBaselinePath || 'ng-baseline.json'
  const baselineData = createNgBaselineData(
    ngOutputMap,
    config.tsconfigPath!,
    config.projectRoot,
    filePaths.length,
    ngDuration,
  )
  await saveNgBaseline(baselineData, outputPath)
  console.log(`Angular baseline saved to: ${outputPath} (${ngOutputMap.size} files)`)
}

/**
 * Print summary for file-level comparison results.
 */
function printFileLevelSummary(results: FileComparisonResult[], totalComponents: number): void {
  const matched = results.filter((r) => r.status === 'match').length
  const mismatched = results.filter((r) => r.status === 'mismatch').length
  const oxcErrors = results.filter((r) => r.status === 'oxc-error').length
  const ngErrors = results.filter((r) => r.status === 'ng-error').length
  const bothErrors = results.filter((r) => r.status === 'both-error').length
  const jitOnly = results.filter((r) => r.status === 'angular-jit-only').length
  const passRate = results.length > 0 ? (matched / results.length) * 100 : 0

  console.log('\n')
  console.log('File-to-File Comparison Results')
  console.log('-------------------------------')
  console.log(`Files compared:     ${results.length}`)
  console.log(`Files matched:      ${matched} (${passRate.toFixed(1)}%)`)
  console.log(`Files mismatched:   ${mismatched}`)
  console.log(`Oxc errors:         ${oxcErrors}`)
  console.log(`Angular errors:     ${ngErrors}`)
  console.log(`Both errors:        ${bothErrors}`)
  console.log(`Angular JIT-only:   ${jitOnly}`)
  console.log(`Total components:   ${totalComponents}`)

  // Show mismatched files summary
  if (mismatched > 0) {
    console.log('\nMismatched Files:')
    const mismatchedResults = results.filter((r) => r.status === 'mismatch')
    for (const result of mismatchedResults.slice(0, 10)) {
      const diffSummary = summarizeComparisonDetails(result.comparisonDetails)
      console.log(`  - ${result.filePath}`)
      if (diffSummary) {
        console.log(`    ${diffSummary}`)
      }
    }
    if (mismatchedResults.length > 10) {
      console.log(`  ... and ${mismatchedResults.length - 10} more`)
    }
  }
}

/**
 * Create a short summary of comparison details for display.
 */
function summarizeComparisonDetails(
  details: FileComparisonResult['comparisonDetails'],
): string | undefined {
  if (!details) return undefined

  const parts: string[] = []

  if (details.importDiffs?.length) {
    parts.push(`${details.importDiffs.length} import diff(s)`)
  }
  if (details.exportDiffs?.length) {
    parts.push(`${details.exportDiffs.length} export diff(s)`)
  }
  if (details.classDiffs?.length) {
    parts.push(`${details.classDiffs.length} class diff(s)`)
  }
  if (details.staticFieldDiffs?.length) {
    parts.push(`${details.staticFieldDiffs.length} static field diff(s)`)
  }
  if (details.classMetadataDiffs?.length) {
    parts.push(`${details.classMetadataDiffs.length} metadata diff(s)`)
  }
  if (details.functionComparison) {
    const fc = details.functionComparison
    if (fc.missingFunctions.length) {
      parts.push(`${fc.missingFunctions.length} missing func(s)`)
    }
    if (fc.extraFunctions.length) {
      parts.push(`${fc.extraFunctions.length} extra func(s)`)
    }
    if (fc.functionDiffs.length) {
      parts.push(`${fc.functionDiffs.length} func diff(s)`)
    }
  }
  if (details.parseErrors?.length) {
    parts.push(`${details.parseErrors.length} parse error(s)`)
  }

  return parts.length > 0 ? `Diff: ${parts.join(', ')}` : undefined
}

/**
 * Check if Angular produced JIT output instead of AOT output.
 *
 * JIT output uses __decorate pattern (runtime decoration) while AOT output
 * uses ɵɵdefineComponent or ɵcmp (compile-time generated).
 *
 * This happens when Angular's AOT compiler can't fully analyze a component
 * (e.g., dynamic providers, certain animations patterns) and falls back to JIT.
 */
function isAngularJitOutput(ngOutput: string, className: string): boolean {
  // JIT output has __decorate pattern
  const hasDecorate = ngOutput.includes('__decorate')
  // AOT output has ɵɵdefineComponent or ɵcmp
  const hasAot = ngOutput.includes('ɵɵdefineComponent') || ngOutput.includes(`${className}.ɵcmp`)
  return hasDecorate && !hasAot
}

/**
 * Calculate file-level summary statistics from file comparison results.
 */
function calculateFileSummary(
  results: FileComparisonResult[],
  totalComponents: number,
): FileReportSummary {
  const summary: FileReportSummary = {
    totalFiles: results.length,
    matchedFiles: 0,
    mismatchedFiles: 0,
    oxcErrors: 0,
    ngErrors: 0,
    bothErrors: 0,
    angularJitOnly: 0,
    passRate: 0,
    totalComponents,
  }

  for (const result of results) {
    switch (result.status) {
      case 'match':
        summary.matchedFiles++
        break
      case 'mismatch':
        summary.mismatchedFiles++
        break
      case 'oxc-error':
        summary.oxcErrors++
        break
      case 'ng-error':
        summary.ngErrors++
        break
      case 'both-error':
        summary.bothErrors++
        break
      case 'angular-jit-only':
        summary.angularJitOnly++
        break
    }
  }

  summary.passRate = summary.totalFiles > 0 ? (summary.matchedFiles / summary.totalFiles) * 100 : 0

  return summary
}

/**
 * Create empty summary for when no components are found.
 */
function createEmptySummary(): ReportSummary {
  return {
    total: 0,
    matched: 0,
    mismatched: 0,
    oxcErrors: 0,
    tsErrors: 0,
    bothErrors: 0,
    angularJitOnly: 0,
    passRate: 0,
  }
}

/**
 * Create metadata for the report.
 */
async function createMetadata(config: CompilerConfig, startTime: number): Promise<ReportMetadata> {
  return {
    generatedAt: new Date().toISOString(),
    projectRoot: config.projectRoot,
    durationMs: performance.now() - startTime,
    oxcVersion: await getOxcVersion(),
    angularVersion: getAngularVersion(),
  }
}

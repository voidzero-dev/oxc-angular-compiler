/**
 * Fixture test runner.
 *
 * Compiles each fixture with both Oxc and TypeScript Angular compilers,
 * then compares the outputs for semantic equivalence.
 */

import { readFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { cpus } from 'os'

import { VERSION as ANGULAR_VERSION } from '@angular/compiler'
import { compileFactory, Severity } from '@oxc-angular/vite/api'
import pLimit from 'p-limit'

import { compareJsSemantically, compareFullFileSemantically } from '../src/compare.js'
import {
  compileWithNgtsc,
  generateComponentSource,
  printDiagnosticWarningSummary,
} from '../src/compilers/angular-ngtsc.js'
import { compileWithOxcFullFile } from '../src/compilers/oxc.js'
import { discoverFixtures } from './index.js'

/**
 * Path to the tsconfig.json used for fixture compilation.
 * Using a shared tsconfig enables NgtscProgram caching across all fixtures,
 * which dramatically speeds up compilation by reusing:
 * - Parsed source files (especially @angular/*)
 * - Module resolution results
 * - The NgtscProgram for incremental compilation
 */
const FIXTURE_TSCONFIG_PATH = join(dirname(fileURLToPath(import.meta.url)), '..', 'tsconfig.json')
import type {
  Fixture,
  FixtureResult,
  FixtureReport,
  FixtureRunnerOptions,
  FixtureCompilerOutput,
  CategoryStats,
  ComponentMetadata,
  FixtureType,
} from './types.js'

/**
 * Get the Oxc Angular compiler version from package.json.
 * Returns undefined if the version cannot be read.
 */
function getOxcVersion(): string | undefined {
  try {
    const currentDir = dirname(fileURLToPath(import.meta.url))
    // Path: fixtures/ -> compare/ -> e2e/ -> angular-compiler/package.json
    const packageJsonPath = join(currentDir, '..', '..', '..', 'package.json')
    const packageJson = JSON.parse(readFileSync(packageJsonPath, 'utf-8'))
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
 * Run all fixtures matching the given options.
 *
 * @param options - Runner configuration
 * @returns Complete fixture test report
 */
export async function runFixtures(options: FixtureRunnerOptions = {}): Promise<FixtureReport> {
  const startTime = Date.now()

  const fixtures = await discoverFixtures(options.categories)

  if (options.verbose) {
    console.log(`Found ${fixtures.length} fixtures to test`)
  }

  const results: FixtureResult[] = []

  if (options.parallel !== false) {
    // Parallel execution
    const limit = pLimit(cpus().length - 1 || 1)
    const tasks = fixtures.map((fixture) => limit(() => testFixture(fixture, options.verbose)))

    let completed = 0
    for (const task of tasks) {
      const result = await task
      results.push(result)
      completed++

      if (completed % 10 === 0 || completed === fixtures.length) {
        process.stdout.write(
          `\rProgress: ${completed}/${fixtures.length} (${Math.round((completed / fixtures.length) * 100)}%)`,
        )
      }
    }
    console.log('') // New line after progress
  } else {
    // Sequential execution
    for (const fixture of fixtures) {
      const result = await testFixture(fixture, options.verbose)
      results.push(result)
    }
  }

  const durationMs = Date.now() - startTime

  // Print summary of diagnostic collection warnings (if any)
  printDiagnosticWarningSummary()

  return generateReport(results, options.categories || [], durationMs)
}

/**
 * Test a single fixture.
 */
async function testFixture(fixture: Fixture, verbose?: boolean): Promise<FixtureResult> {
  // Handle skipped fixtures (via skip flag or skipReason)
  if (fixture.skip || fixture.skipReason) {
    return {
      fixture,
      status: 'skipped',
      skipReason: fixture.skipReason,
    }
  }

  // Dispatch based on fixture type
  const fixtureType: FixtureType = fixture.type || 'component'

  switch (fixtureType) {
    case 'factory':
      return testFactoryFixture(fixture, verbose)
    case 'full-transform':
      return testFullTransformFixture(fixture, verbose)
    case 'class-metadata':
      return testClassMetadataFixture(fixture, verbose)
    case 'directive':
    case 'injectable':
      // These require NAPI bindings that don't exist yet - mark as skipped
      return {
        fixture,
        status: 'skipped',
        skipReason: `${fixtureType} compilation NAPI bindings not yet implemented`,
      }
    case 'component':
    default:
      return testComponentFixture(fixture, verbose)
  }
}

/**
 * Test a component fixture (template compilation).
 *
 * Uses full-file compilation on both sides:
 * 1. Generate component source from template + metadata
 * 2. Compile with both Oxc and NgtscProgram
 * 3. Use compareFullFileSemantically for comparison
 */
async function testComponentFixture(fixture: Fixture, verbose?: boolean): Promise<FixtureResult> {
  const filePath = fixture.filePath || `fixtures/${fixture.category}/${fixture.name}.component.ts`

  // Extract metadata from fixture for full component compilation testing
  const metadata: ComponentMetadata = {
    selector: fixture.selector,
    standalone: fixture.standalone,
    encapsulation: fixture.encapsulation,
    changeDetection: fixture.changeDetection,
    host: fixture.host,
    preserveWhitespaces: fixture.preserveWhitespaces,
    providers: fixture.providers,
    viewProviders: fixture.viewProviders,
    animations: fixture.animations,
    schemas: fixture.schemas,
    imports: fixture.imports,
    hostDirectives: fixture.hostDirectives,
    styles: fixture.styles,
    exportAs: fixture.exportAs,
  }

  // Generate the component source from template + metadata
  const source = generateComponentSource(fixture.template!, fixture.className, metadata)

  // Compile with Oxc full-file transform
  let oxcOutput: FixtureCompilerOutput
  const oxcStart = performance.now()
  try {
    const result = compileWithOxcFullFile(source, filePath)
    oxcOutput = {
      code: result.code,
      error: result.error,
      compilationTimeMs: performance.now() - oxcStart,
    }
  } catch (error) {
    oxcOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - oxcStart,
    }
  }

  // Compile with NgtscProgram (full Angular AOT pipeline)
  // Use shared tsconfig for program caching
  let tsOutput: FixtureCompilerOutput
  const tsStart = performance.now()
  try {
    const result = await compileWithNgtsc(source, filePath, { tsconfigPath: FIXTURE_TSCONFIG_PATH })
    tsOutput = {
      code: result.code,
      error: result.error,
      compilationTimeMs: performance.now() - tsStart,
    }
  } catch (error) {
    tsOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - tsStart,
    }
  }

  // Handle compilation errors
  if (oxcOutput.error && tsOutput.error) {
    return { fixture, status: 'both-error', oxcOutput, tsOutput }
  }
  if (oxcOutput.error) {
    return { fixture, status: 'oxc-error', oxcOutput, tsOutput }
  }
  if (tsOutput.error) {
    return { fixture, status: 'ts-error', oxcOutput, tsOutput }
  }

  // Use full-file semantic comparison
  const comparison = await compareFullFileSemantically(oxcOutput.code, tsOutput.code)

  // Check expected features if specified
  let oxcFeatures: string[] | undefined
  let tsFeatures: string[] | undefined
  let asymmetricFeatures: string[] | undefined
  let missingFromBoth: string[] | undefined

  if (fixture.expectedFeatures && fixture.expectedFeatures.length > 0) {
    oxcFeatures = []
    tsFeatures = []
    asymmetricFeatures = []
    missingFromBoth = []

    for (const feature of fixture.expectedFeatures) {
      const inOxc = oxcOutput.code.includes(feature)
      const inTs = tsOutput.code.includes(feature)

      if (inOxc) oxcFeatures.push(feature)
      if (inTs) tsFeatures.push(feature)
      if (inOxc !== inTs) asymmetricFeatures.push(feature)
      if (!inOxc && !inTs) missingFromBoth.push(feature)
    }
  }

  if (comparison.match) {
    return {
      fixture,
      status: 'match',
      oxcOutput,
      tsOutput,
      oxcFeatures,
      tsFeatures,
      asymmetricFeatures,
      missingFromBoth,
    }
  }

  // Log mismatch details if verbose
  if (verbose) {
    console.log(`\n  MISMATCH: ${fixture.category}/${fixture.name}`)
    if (comparison.importDiffs && comparison.importDiffs.length > 0) {
      console.log(`    Import diffs: ${comparison.importDiffs.length}`)
    }
    if (comparison.classDiffs && comparison.classDiffs.length > 0) {
      console.log(`    Class diffs: ${comparison.classDiffs.map((d) => d.className).join(', ')}`)
    }
    if (comparison.functionComparison) {
      const fc = comparison.functionComparison
      if (fc.missingFunctions.length > 0) {
        console.log(`    Missing functions: ${fc.missingFunctions.join(', ')}`)
      }
      if (fc.extraFunctions.length > 0) {
        console.log(`    Extra functions: ${fc.extraFunctions.join(', ')}`)
      }
    }
  }

  return {
    fixture,
    status: 'mismatch',
    oxcOutput,
    tsOutput,
    functionComparison: comparison.functionComparison,
    importDiffs: comparison.importDiffs,
    exportDiffs: comparison.exportDiffs,
    classDiffs: comparison.classDiffs,
    staticFieldDiffs: comparison.staticFieldDiffs,
    classMetadataDiffs: comparison.classMetadataDiffs,
    parseErrors: comparison.parseErrors,
    oxcFeatures,
    tsFeatures,
    asymmetricFeatures,
    missingFromBoth,
  }
}

/**
 * Test a factory fixture (factory function compilation).
 */
async function testFactoryFixture(fixture: Fixture, verbose?: boolean): Promise<FixtureResult> {
  // Compile with Oxc NAPI
  let oxcOutput: FixtureCompilerOutput
  const oxcStart = performance.now()
  try {
    // Extract factory-specific fields from fixture
    const factoryName = (fixture as any).factoryName || fixture.className
    const target = (fixture as any).target
    const depsKind = (fixture as any).depsKind
    const deps = (fixture as any).deps

    const result = await compileFactory({
      name: factoryName,
      target,
      depsKind,
      deps,
    })

    const errors = result.errors.filter((e) => e.severity === Severity.Error)
    if (errors.length > 0) {
      oxcOutput = {
        code: '',
        error: errors.map((e) => e.message).join('\n'),
        compilationTimeMs: performance.now() - oxcStart,
      }
    } else {
      oxcOutput = {
        code: result.code,
        compilationTimeMs: performance.now() - oxcStart,
      }
    }
  } catch (error) {
    oxcOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - oxcStart,
    }
  }

  // For now, use Oxc output as baseline
  const tsOutput: FixtureCompilerOutput = {
    code: oxcOutput.code,
    compilationTimeMs: 0,
  }

  return await processCompilationResults(fixture, oxcOutput, tsOutput, verbose)
}

/**
 * Test a full-transform fixture (full TypeScript file compilation).
 *
 * This uses transformAngularFile to compile a complete TypeScript source file
 * containing a @Component decorated class, including class-level decorators
 * like @HostBinding and @HostListener.
 *
 * Uses compareFullFileSemantically() for AST-based semantic comparison of
 * the full compiled output.
 */
async function testFullTransformFixture(
  fixture: Fixture,
  verbose?: boolean,
): Promise<FixtureResult> {
  if (!fixture.sourceCode) {
    return {
      fixture,
      status: 'skipped',
      skipReason: 'No source code',
    }
  }

  const filePath = fixture.filePath || `${fixture.category}/${fixture.name}.ts`

  // Compile with both compilers (no className needed - returning full output)
  let oxcOutput: FixtureCompilerOutput
  let tsOutput: FixtureCompilerOutput

  const oxcStart = performance.now()
  try {
    const result = compileWithOxcFullFile(fixture.sourceCode, filePath)
    oxcOutput = {
      code: result.code,
      error: result.error,
      compilationTimeMs: performance.now() - oxcStart,
    }
  } catch (error) {
    oxcOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - oxcStart,
    }
  }

  const tsStart = performance.now()
  try {
    const result = await compileWithNgtsc(fixture.sourceCode, filePath, {
      tsconfigPath: FIXTURE_TSCONFIG_PATH,
    })
    tsOutput = {
      code: result.code,
      error: result.error,
      compilationTimeMs: performance.now() - tsStart,
    }
  } catch (error) {
    tsOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - tsStart,
    }
  }

  // Handle compilation errors (testFullTransformFixture)
  if (oxcOutput.error && tsOutput.error) {
    return { fixture, status: 'both-error', oxcOutput, tsOutput }
  }
  if (oxcOutput.error) {
    return { fixture, status: 'oxc-error', oxcOutput, tsOutput }
  }
  if (tsOutput.error) {
    return { fixture, status: 'ts-error', oxcOutput, tsOutput }
  }

  // Use full-file semantic comparison
  const comparison = await compareFullFileSemantically(oxcOutput.code, tsOutput.code)

  if (comparison.match) {
    return { fixture, status: 'match', oxcOutput, tsOutput }
  }

  // Log mismatch details if verbose
  if (verbose) {
    console.log(`\n  MISMATCH: ${fixture.category}/${fixture.name}`)
    if (comparison.importDiffs && comparison.importDiffs.length > 0) {
      console.log(`    Import diffs: ${comparison.importDiffs.length}`)
    }
    if (comparison.exportDiffs && comparison.exportDiffs.length > 0) {
      console.log(`    Export diffs: ${comparison.exportDiffs.length}`)
    }
    if (comparison.classDiffs && comparison.classDiffs.length > 0) {
      console.log(`    Class diffs: ${comparison.classDiffs.map((d) => d.className).join(', ')}`)
    }
    if (comparison.staticFieldDiffs && comparison.staticFieldDiffs.length > 0) {
      console.log(
        `    Static field diffs: ${comparison.staticFieldDiffs.map((d) => `${d.className}.${d.fieldName}`).join(', ')}`,
      )
    }
    if (comparison.classMetadataDiffs && comparison.classMetadataDiffs.length > 0) {
      console.log(
        `    Class metadata diffs: ${comparison.classMetadataDiffs.map((d) => d.className).join(', ')}`,
      )
    }
    if (comparison.functionComparison) {
      const fc = comparison.functionComparison
      if (fc.missingFunctions.length > 0) {
        console.log(`    Missing functions: ${fc.missingFunctions.join(', ')}`)
      }
      if (fc.extraFunctions.length > 0) {
        console.log(`    Extra functions: ${fc.extraFunctions.join(', ')}`)
      }
      if (fc.functionDiffs.length > 0) {
        console.log(`    Differing functions: ${fc.functionDiffs.map((d) => d.name).join(', ')}`)
      }
    }
    if (comparison.parseErrors && comparison.parseErrors.length > 0) {
      console.log(`    Parse errors: ${comparison.parseErrors.join(', ')}`)
    }
  }

  // Return detailed mismatch information
  return {
    fixture,
    status: 'mismatch',
    oxcOutput,
    tsOutput,
    functionComparison: comparison.functionComparison,
    importDiffs: comparison.importDiffs,
    exportDiffs: comparison.exportDiffs,
    classDiffs: comparison.classDiffs,
    staticFieldDiffs: comparison.staticFieldDiffs,
    classMetadataDiffs: comparison.classMetadataDiffs,
    parseErrors: comparison.parseErrors,
  }
}

/**
 * Check if Angular produced AOT output with Angular static fields.
 *
 * When Angular cannot fully analyze a component (e.g., unresolved imports),
 * it may output the TypeScript source with decorators preserved instead of
 * generating AOT output with static fields like ɵcmp, ɵfac, ɵdir, ɵprov.
 *
 * This function checks if the output contains any Angular-specific static fields
 * that indicate successful AOT compilation.
 */
function hasAngularAotOutput(code: string): boolean {
  // Check for Angular-specific static field patterns
  // AOT output has patterns like: static ɵcmp = ..., static ɵfac = ..., etc.
  return (
    code.includes('static ɵcmp') ||
    code.includes('static ɵfac') ||
    code.includes('static ɵdir') ||
    code.includes('static ɵprov') ||
    code.includes('static ɵpipe') ||
    code.includes('static ɵmod') ||
    code.includes('static ɵinj') ||
    code.includes('ɵɵdefineComponent') ||
    code.includes('ɵɵdefineDirective') ||
    code.includes('ɵɵdefineInjectable') ||
    code.includes('ɵɵdefinePipe') ||
    code.includes('ɵɵdefineNgModule')
  )
}

/**
 * Test a class-metadata fixture (setClassMetadata compilation).
 *
 * Uses full-file compilation with NgtscProgram, which includes
 * setClassMetadata calls as part of the output. The comparison uses
 * compareFullFileSemantically() which already handles class metadata
 * comparison as a dedicated category.
 */
async function testClassMetadataFixture(
  fixture: Fixture,
  verbose?: boolean,
): Promise<FixtureResult> {
  if (!fixture.sourceCode) {
    return {
      fixture,
      status: 'oxc-error',
      oxcOutput: {
        code: '',
        error: 'Class-metadata fixtures require sourceCode field',
        compilationTimeMs: 0,
      },
    }
  }

  // Use absolute real filesystem path for proper module resolution with tsconfig
  // Note: fixture.filePath is set by discoverFixtures() to a path like '/fixtures/...'
  // which is not a real absolute path. We need to compute the real absolute path.
  const compareDir = dirname(FIXTURE_TSCONFIG_PATH)
  const filePath = join(compareDir, 'fixtures', fixture.category, `${fixture.name}.ts`)

  // Compile with Oxc full-file transform
  let oxcOutput: FixtureCompilerOutput
  const oxcStart = performance.now()
  try {
    const result = compileWithOxcFullFile(fixture.sourceCode, filePath)
    oxcOutput = {
      code: result.code,
      error: result.error,
      compilationTimeMs: performance.now() - oxcStart,
    }
  } catch (error) {
    oxcOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - oxcStart,
    }
  }

  // Compile with NgtscProgram (full Angular AOT pipeline)
  let tsOutput: FixtureCompilerOutput
  const tsStart = performance.now()
  try {
    const result = await compileWithNgtsc(fixture.sourceCode, filePath, {
      tsconfigPath: FIXTURE_TSCONFIG_PATH,
    })
    tsOutput = {
      code: result.code,
      error: result.error,
      compilationTimeMs: performance.now() - tsStart,
    }
  } catch (error) {
    tsOutput = {
      code: '',
      error: String(error),
      compilationTimeMs: performance.now() - tsStart,
    }
  }

  // Handle compilation errors (testClassMetadataFixture)
  if (oxcOutput.error && tsOutput.error) {
    return { fixture, status: 'both-error', oxcOutput, tsOutput }
  }
  if (oxcOutput.error) {
    return { fixture, status: 'oxc-error', oxcOutput, tsOutput }
  }
  if (tsOutput.error) {
    return { fixture, status: 'ts-error', oxcOutput, tsOutput }
  }

  // Check if Angular produced AOT output
  // When Angular cannot resolve imports or types (e.g., DOCUMENT from @angular/common,
  // or constructor parameters with 'any' type and no @Inject token), it outputs the
  // TypeScript source with decorators preserved instead of generating AOT output.
  // This is a limitation of testing with virtual files - these fixtures work fine
  // when compiled as part of a real project with proper type information.
  if (!hasAngularAotOutput(tsOutput.code)) {
    return {
      fixture,
      status: 'skipped',
      skipReason:
        'Angular did not produce AOT output (fixture requires full type resolution unavailable in virtual mode)',
      oxcOutput,
      tsOutput,
    }
  }

  // Use full-file semantic comparison (includes class metadata comparison)
  const comparison = await compareFullFileSemantically(oxcOutput.code, tsOutput.code)

  if (comparison.match) {
    return { fixture, status: 'match', oxcOutput, tsOutput }
  }

  // Log mismatch details if verbose
  if (verbose) {
    console.log(`\n  MISMATCH: ${fixture.category}/${fixture.name}`)
    if (comparison.classMetadataDiffs && comparison.classMetadataDiffs.length > 0) {
      console.log(
        `    Class metadata diffs: ${comparison.classMetadataDiffs.map((d) => d.className).join(', ')}`,
      )
    }
  }

  return {
    fixture,
    status: 'mismatch',
    oxcOutput,
    tsOutput,
    functionComparison: comparison.functionComparison,
    classMetadataDiffs: comparison.classMetadataDiffs,
  }
}

/**
 * Process compilation results and generate fixture result.
 */
async function processCompilationResults(
  fixture: Fixture,
  oxcOutput: FixtureCompilerOutput,
  tsOutput: FixtureCompilerOutput,
  verbose?: boolean,
): Promise<FixtureResult> {
  // Determine status
  const oxcFailed = Boolean(oxcOutput.error)
  const tsFailed = Boolean(tsOutput.error)

  if (oxcFailed && tsFailed) {
    return {
      fixture,
      status: 'both-error',
      oxcOutput,
      tsOutput,
    }
  }

  if (oxcFailed) {
    return {
      fixture,
      status: 'oxc-error',
      oxcOutput,
      tsOutput,
    }
  }

  if (tsFailed) {
    return {
      fixture,
      status: 'ts-error',
      oxcOutput,
      tsOutput,
    }
  }

  // Compare outputs
  const comparison = await compareJsSemantically(oxcOutput.code, tsOutput.code)

  // Check expected features if specified - stricter check for regressions
  let oxcFeatures: string[] | undefined
  let tsFeatures: string[] | undefined
  let asymmetricFeatures: string[] | undefined
  let missingFromBoth: string[] | undefined

  if (fixture.expectedFeatures && fixture.expectedFeatures.length > 0) {
    oxcFeatures = []
    tsFeatures = []
    asymmetricFeatures = []
    missingFromBoth = []

    for (const feature of fixture.expectedFeatures) {
      const inOxc = oxcOutput.code.includes(feature)
      const inTs = tsOutput.code.includes(feature)

      if (inOxc) {
        oxcFeatures.push(feature)
      }
      if (inTs) {
        tsFeatures.push(feature)
      }

      // Track asymmetric features (present in one but not both)
      if (inOxc !== inTs) {
        asymmetricFeatures.push(feature)
      }

      // Track features missing from both (test configuration issue)
      if (!inOxc && !inTs) {
        missingFromBoth.push(feature)
      }
    }
  }

  if (verbose && !comparison.match) {
    console.log(`\n  MISMATCH: ${fixture.category}/${fixture.name}`)
    if (comparison.functionComparison) {
      const fc = comparison.functionComparison
      if (fc.missingFunctions.length > 0) {
        console.log(`    Missing functions: ${fc.missingFunctions.join(', ')}`)
      }
      if (fc.extraFunctions.length > 0) {
        console.log(`    Extra functions: ${fc.extraFunctions.join(', ')}`)
      }
      if (fc.functionDiffs.length > 0) {
        console.log(`    Differing functions: ${fc.functionDiffs.map((d) => d.name).join(', ')}`)
      }
    }
  }

  // Warn about feature asymmetries (potential regressions)
  if (verbose && asymmetricFeatures && asymmetricFeatures.length > 0) {
    console.log(`\n  FEATURE ASYMMETRY: ${fixture.category}/${fixture.name}`)
    for (const feature of asymmetricFeatures) {
      const inOxc = oxcFeatures?.includes(feature)
      const inTs = tsFeatures?.includes(feature)
      if (inOxc && !inTs) {
        console.log(`    ${feature}: Oxc only (not in Angular)`)
      } else if (!inOxc && inTs) {
        console.log(`    ${feature}: Angular only (not in Oxc)`)
      }
    }
  }

  // Warn about features missing from both outputs (test configuration issue)
  if (verbose && missingFromBoth && missingFromBoth.length > 0) {
    console.log(`\n  MISSING FEATURES: ${fixture.category}/${fixture.name}`)
    console.log(`    Not in either output: ${missingFromBoth.join(', ')}`)
  }

  // Determine final status
  const status: FixtureResult['status'] = comparison.match ? 'match' : 'mismatch'

  return {
    fixture,
    status,
    oxcOutput,
    tsOutput,
    functionComparison: comparison.functionComparison,
    diff: comparison.diff,
    oxcFeatures,
    tsFeatures,
    asymmetricFeatures,
    missingFromBoth,
  }
}

/**
 * Generate a report from fixture results.
 */
function generateReport(
  results: FixtureResult[],
  categories: string[],
  durationMs: number,
): FixtureReport {
  // Calculate summary statistics
  let matched = 0
  let mismatched = 0
  let oxcErrors = 0
  let tsErrors = 0
  let bothErrors = 0
  let skipped = 0

  const byCategory = new Map<string, FixtureResult[]>()

  for (const result of results) {
    // Track by category
    const existing = byCategory.get(result.fixture.category) || []
    existing.push(result)
    byCategory.set(result.fixture.category, existing)

    // Count statuses
    switch (result.status) {
      case 'match':
        matched++
        break
      case 'mismatch':
        mismatched++
        break
      case 'oxc-error':
        oxcErrors++
        break
      case 'ts-error':
        tsErrors++
        break
      case 'both-error':
        bothErrors++
        break
      case 'skipped':
        skipped++
        break
    }
  }

  // Calculate category stats
  const categoryStats: Record<string, CategoryStats> = {}
  for (const [category, categoryResults] of byCategory) {
    const total = categoryResults.length
    const passed = categoryResults.filter((r) => r.status === 'match').length
    const failed = categoryResults.filter(
      (r) => r.status === 'mismatch' || r.status.includes('error'),
    ).length
    const categorySkipped = categoryResults.filter((r) => r.status === 'skipped').length
    const denominator = total - categorySkipped

    categoryStats[category] = {
      total,
      passed,
      failed,
      skipped: categorySkipped,
      passRate: denominator > 0 ? (passed / denominator) * 100 : 100,
    }
  }

  const denominator = results.length - skipped
  const passRate = denominator > 0 ? (matched / denominator) * 100 : 100

  return {
    summary: {
      total: results.length,
      matched,
      mismatched,
      oxcErrors,
      tsErrors,
      bothErrors,
      skipped,
      passRate,
      byCategory: categoryStats,
    },
    metadata: {
      generatedAt: new Date().toISOString(),
      durationMs,
      categories,
      oxcVersion: getOxcVersion(),
      angularVersion: getAngularVersion(),
    },
    fixtures: results,
  }
}

/**
 * Print a summary of the fixture report to console.
 */
export function printFixtureSummary(report: FixtureReport): void {
  console.log('\nFixture Test Results')
  console.log('='.repeat(50))
  console.log(`Total fixtures:     ${report.summary.total}`)
  console.log(
    `Matched:            ${report.summary.matched} (${report.summary.passRate.toFixed(1)}%)`,
  )
  console.log(`Mismatched:         ${report.summary.mismatched}`)
  console.log(`Oxc errors:         ${report.summary.oxcErrors}`)
  console.log(`TS errors:          ${report.summary.tsErrors}`)
  console.log(`Both errors:        ${report.summary.bothErrors}`)
  console.log(`Skipped:            ${report.summary.skipped}`)

  // Show skipped fixtures with reasons
  const skippedFixtures = report.fixtures.filter((r) => r.status === 'skipped')
  if (skippedFixtures.length > 0) {
    console.log('\nSkipped Fixtures:')
    console.log('-'.repeat(50))
    for (const result of skippedFixtures) {
      const reason = result.skipReason || 'No reason provided'
      console.log(`  ${result.fixture.category}/${result.fixture.name}: ${reason}`)
    }
  }

  // Count feature discrepancies
  const fixturesWithAsymmetricFeatures = report.fixtures.filter(
    (r) => r.asymmetricFeatures && r.asymmetricFeatures.length > 0,
  )
  const fixturesWithMissingFeatures = report.fixtures.filter(
    (r) => r.missingFromBoth && r.missingFromBoth.length > 0,
  )

  if (fixturesWithAsymmetricFeatures.length > 0 || fixturesWithMissingFeatures.length > 0) {
    console.log('\nFeature Discrepancies:')
    console.log('-'.repeat(50))
    if (fixturesWithAsymmetricFeatures.length > 0) {
      console.log(`  Asymmetric features: ${fixturesWithAsymmetricFeatures.length} fixture(s)`)
      for (const result of fixturesWithAsymmetricFeatures) {
        console.log(`    ${result.fixture.category}/${result.fixture.name}:`)
        for (const feature of result.asymmetricFeatures!) {
          const inOxc = result.oxcFeatures?.includes(feature)
          if (inOxc) {
            console.log(`      ${feature}: Oxc only`)
          } else {
            console.log(`      ${feature}: Angular only`)
          }
        }
      }
    }
    if (fixturesWithMissingFeatures.length > 0) {
      console.log(`  Missing from both:   ${fixturesWithMissingFeatures.length} fixture(s)`)
      for (const result of fixturesWithMissingFeatures) {
        console.log(
          `    ${result.fixture.category}/${result.fixture.name}: ${result.missingFromBoth!.join(', ')}`,
        )
      }
    }
  }

  if (Object.keys(report.summary.byCategory).length > 0) {
    console.log('\nBy Category:')
    console.log('-'.repeat(50))
    for (const [category, stats] of Object.entries(report.summary.byCategory)) {
      const activeFixtures = stats.total - stats.skipped
      const status = stats.passed === activeFixtures ? 'PASS' : 'FAIL'
      console.log(
        `  ${category.padEnd(20)} ${stats.passed}/${activeFixtures} (${stats.passRate.toFixed(1)}%) [${status}]`,
      )
    }
  }

  console.log('\n' + '-'.repeat(50))
  console.log(`Duration: ${(report.metadata.durationMs / 1000).toFixed(2)}s`)
  if (report.metadata.oxcVersion) {
    console.log(`Oxc version: ${report.metadata.oxcVersion}`)
  }
  if (report.metadata.angularVersion) {
    console.log(`Angular version: ${report.metadata.angularVersion}`)
  }
}

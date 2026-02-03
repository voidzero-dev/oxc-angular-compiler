#!/usr/bin/env node
/**
 * Angular Compiler Comparison CLI
 *
 * Compares the Oxc Angular compiler output with the TypeScript Angular compiler
 * for real-world Angular projects and/or synthetic fixtures.
 */

import { writeFile } from 'fs/promises'
import path, { resolve } from 'path'
import { parseArgs } from 'util'

import { diffLines } from 'diff'

import { listFixtures } from '../fixtures/index.js'
import { runFixtures, printFixtureSummary } from '../fixtures/runner.js'
import { formatPresetList, getPreset, getPresetNames, mergePresetWithCli } from './presets.js'
import { runComparison } from './runner.js'
import type { CompilerConfig, CompilationResult, ComparisonReport } from './types.js'

const { values, positionals } = parseArgs({
  args: process.argv.slice(2),
  allowPositionals: true,
  options: {
    project: {
      type: 'string',
      short: 'p',
      default: '.',
      description: 'Project root directory',
    },
    output: {
      type: 'string',
      short: 'o',
      default: './compare-report.json',
      description: 'Output JSON report path',
    },
    include: {
      type: 'string',
      multiple: true,
      description: 'Include glob patterns',
    },
    exclude: {
      type: 'string',
      multiple: true,
      description: 'Exclude glob patterns',
    },
    parallel: {
      type: 'boolean',
      default: true,
      description: 'Enable parallel compilation',
    },
    'no-parallel': {
      type: 'boolean',
      description: 'Disable parallel compilation',
    },
    verbose: {
      type: 'boolean',
      short: 'v',
      description: 'Verbose output',
    },
    help: {
      type: 'boolean',
      short: 'h',
      description: 'Show help',
    },
    // Fixture mode options
    fixtures: {
      type: 'boolean',
      description: 'Run fixture tests instead of project comparison',
    },
    both: {
      type: 'boolean',
      description: 'Run both project and fixture tests',
    },
    category: {
      type: 'string',
      multiple: true,
      description: 'Filter fixtures by category (can be repeated)',
    },
    'list-fixtures': {
      type: 'boolean',
      description: 'List all available fixtures without running',
    },
    // Preset options
    preset: {
      type: 'string',
      description: 'Use a predefined configuration preset',
    },
    'list-presets': {
      type: 'boolean',
      description: 'List all available presets without running',
    },
    // Compilation mode options
    'full-file': {
      type: 'boolean',
      description: 'Use full-file compilation mode instead of template-only',
    },
    // Angular baseline options
    'ng-baseline': {
      type: 'string',
      description: 'Path to Angular baseline file (skip Angular compilation)',
    },
    'save-ng-baseline': {
      type: 'string',
      description: 'Save Angular compilation output to baseline file',
    },
    'generate-ng-baseline': {
      type: 'boolean',
      description: 'Run Angular compilation only and save baseline (no Oxc, no comparison)',
    },
    // Output options
    'json-reporter': {
      type: 'boolean',
      description: 'Output results as JSON to stdout',
    },
    'detailed-diff': {
      type: 'boolean',
      description: 'Show detailed function body diffs for mismatches',
    },
  },
})

if (values.help) {
  printHelp()
  process.exit(0)
}

// Handle --list-presets
if (values['list-presets']) {
  console.log(formatPresetList())
  process.exit(0)
}

// Handle --list-fixtures
if (values['list-fixtures']) {
  const listing = await listFixtures(values.category)
  console.log(listing)
  process.exit(0)
}

// Validate and resolve preset if specified
let presetInclude: string[] | undefined
let presetExclude: string[] | undefined
let presetName: string | undefined
let presetTsconfigPath: string | undefined
let presetProjectRoot: string | undefined

if (values.preset) {
  const preset = getPreset(values.preset)
  if (!preset) {
    console.error(`Error: Unknown preset "${values.preset}"`)
    console.error(`Available presets: ${getPresetNames().join(', ')}`)
    process.exit(1)
  }
  presetName = preset.name
  const merged = mergePresetWithCli(preset, values.include, values.exclude)
  presetInclude = merged.include
  presetExclude = merged.exclude
  // Pass tsconfigPath for full-file compilation mode
  presetTsconfigPath = preset.tsconfigPath
  // Store preset's projectRoot for later use
  presetProjectRoot = preset.projectRoot ? resolve(preset.projectRoot) : undefined
}

// Determine run mode
const runFixturesOnly = values.fixtures && !values.both
const runBoth = values.both
const runProjectOnly = !values.fixtures && !values.both

try {
  let hasFailures = false

  // Run fixtures if requested
  if (runFixturesOnly || runBoth) {
    console.log('Running fixture tests...\n')
    const fixtureReport = await runFixtures({
      categories: values.category,
      verbose: values.verbose,
      parallel: !values['no-parallel'] && values.parallel !== false,
    })

    printFixtureSummary(fixtureReport)

    // Write fixture report
    const fixtureOutputPath = runBoth
      ? resolve(values.output!.slice(0, -path.extname(values.output!).length) + '-fixtures.json')
      : resolve(values.output!)
    await writeFile(fixtureOutputPath, JSON.stringify(fixtureReport, null, 2), 'utf-8')
    console.log(`\nFixture report written to: ${fixtureOutputPath}`)

    hasFailures =
      hasFailures || fixtureReport.summary.mismatched > 0 || fixtureReport.summary.oxcErrors > 0
  }

  // Run project comparison if requested
  if (runProjectOnly || runBoth) {
    // Use preset's projectRoot if no explicit project path was given
    const hasExplicitProject = positionals.length > 0 || (values.project && values.project !== '.')
    const projectRoot =
      !hasExplicitProject && presetProjectRoot
        ? presetProjectRoot
        : resolve(positionals[0] || values.project || '.')

    const config: CompilerConfig = {
      projectRoot,
      include: presetInclude ?? values.include,
      exclude: presetExclude ?? values.exclude,
      parallel: !values['no-parallel'] && values.parallel !== false,
      outputPath: values.output,
      presetName,
      fullFileMode: values['full-file'],
      // Pass tsconfigPath for full-file compilation with real project context
      tsconfigPath: values['full-file'] ? presetTsconfigPath : undefined,
      // Angular baseline options
      ngBaselinePath: values['ng-baseline'] ? resolve(values['ng-baseline']) : undefined,
      saveNgBaselinePath: values['save-ng-baseline']
        ? resolve(values['save-ng-baseline'])
        : undefined,
      generateNgBaselineOnly: values['generate-ng-baseline'],
    }

    if (runBoth) {
      console.log('\n' + '='.repeat(60))
      console.log('Running project comparison...\n')
    }

    const report = await runComparison(config)

    // In generate-ng-baseline mode, the baseline is already saved by the runner.
    // Skip writing the comparison report since there's no comparison data.
    if (!values['generate-ng-baseline']) {
      // Handle --json-reporter: output JSON to stdout
      if (values['json-reporter']) {
        console.log(JSON.stringify(report, null, 2))
      } else {
        // Write JSON report to file (normal behavior)
        const outputPath = resolve(values.output!)
        await writeFile(outputPath, JSON.stringify(report, null, 2), 'utf-8')
        console.log(`\nReport written to: ${outputPath}`)
      }

      // Handle --detailed-diff: show detailed function body diffs for mismatches
      if (values['detailed-diff']) {
        showDetailedDiffs(report)
      }

      hasFailures = hasFailures || report.summary.mismatched > 0 || report.summary.oxcErrors > 0
    }
  }

  process.exit(hasFailures ? 1 : 0)
} catch (error) {
  console.error('Error:', error)
  process.exit(2)
}

/**
 * ANSI color codes for terminal output.
 */
const colors = {
  reset: '\x1b[0m',
  red: '\x1b[31m',
  green: '\x1b[32m',
  yellow: '\x1b[33m',
  blue: '\x1b[34m',
  cyan: '\x1b[36m',
  dim: '\x1b[2m',
  bold: '\x1b[1m',
}

/**
 * Show detailed function body diffs for mismatched components.
 */
function showDetailedDiffs(report: ComparisonReport): void {
  const mismatched = report.results.filter((r) => r.status === 'mismatch')

  if (mismatched.length === 0) {
    console.log('\nNo mismatches to display.')
    return
  }

  console.log('\n' + '='.repeat(80))
  console.log(
    `${colors.bold}DETAILED DIFFS (${mismatched.length} mismatched components)${colors.reset}`,
  )
  console.log('='.repeat(80))

  for (const result of mismatched) {
    showDetailedDiffForComponent(result)
  }
}

/**
 * Show detailed diff for a single component.
 */
function showDetailedDiffForComponent(result: CompilationResult): void {
  const { component, functionComparison } = result

  console.log('\n' + '-'.repeat(80))
  console.log(`${colors.cyan}File:${colors.reset} ${component.filePath}`)
  console.log(`${colors.cyan}Class:${colors.reset} ${component.className}`)
  console.log('-'.repeat(80))

  if (!functionComparison) {
    console.log(`${colors.yellow}No function-level comparison available.${colors.reset}`)
    return
  }

  // Show missing functions
  if (functionComparison.missingFunctions.length > 0) {
    console.log(`\n${colors.red}Missing functions (in TypeScript but not in Oxc):${colors.reset}`)
    for (const name of functionComparison.missingFunctions) {
      console.log(`  ${colors.red}- ${name}${colors.reset}`)
    }
  }

  // Show extra functions
  if (functionComparison.extraFunctions.length > 0) {
    console.log(`\n${colors.green}Extra functions (in Oxc but not in TypeScript):${colors.reset}`)
    for (const name of functionComparison.extraFunctions) {
      console.log(`  ${colors.green}+ ${name}${colors.reset}`)
    }
  }

  // Show function diffs with line-by-line comparison
  if (functionComparison.functionDiffs.length > 0) {
    console.log(`\n${colors.yellow}Functions with differences:${colors.reset}`)

    for (const funcDiff of functionComparison.functionDiffs) {
      console.log(`\n  ${colors.bold}Function: ${funcDiff.name}${colors.reset}`)

      // Use diffLines to show line-by-line diff
      const changes = diffLines(funcDiff.tsCode, funcDiff.oxcCode)

      for (const change of changes) {
        const lines = change.value.split('\n').filter((l) => l.length > 0)

        for (const line of lines) {
          if (change.added) {
            // Line in Oxc but not in TS (actual)
            console.log(`    ${colors.green}+ ${line}${colors.reset}`)
          } else if (change.removed) {
            // Line in TS but not in Oxc (expected)
            console.log(`    ${colors.red}- ${line}${colors.reset}`)
          } else {
            // Unchanged line (context)
            console.log(`    ${colors.dim}  ${line}${colors.reset}`)
          }
        }
      }
    }
  }

  // Show matching functions count
  if (functionComparison.matchingFunctions.length > 0) {
    console.log(
      `\n${colors.dim}Matching functions: ${functionComparison.matchingFunctions.length}${colors.reset}`,
    )
  }
}

function printHelp(): void {
  console.log(`
Angular Compiler Comparison Tool

Compares the Oxc Angular compiler output with the TypeScript @angular/compiler
for real-world Angular projects and/or standalone fixtures.

USAGE:
  npx tsx src/index.ts [PROJECT_PATH] [OPTIONS]

ARGUMENTS:
  PROJECT_PATH    Path to Angular project (default: current directory)

OPTIONS:
  -p, --project <path>      Project root directory (alternative to positional)
  -o, --output <path>       Output JSON report path (default: ./compare-report.json)
  --include <glob>          Include patterns (can be repeated)
  --exclude <glob>          Exclude patterns (can be repeated)
  --parallel                Enable parallel compilation (default)
  --no-parallel             Disable parallel compilation
  -v, --verbose             Verbose output
  -h, --help                Show this help

FIXTURE OPTIONS:
  --fixtures                Run fixture tests instead of project comparison
  --both                    Run both project and fixture tests
  --category <name>         Filter fixtures by category (can be repeated)
  --list-fixtures           List all available fixtures without running

PRESET OPTIONS:
  --preset <name>           Use a predefined configuration preset
  --list-presets            List all available presets without running

COMPILATION MODE OPTIONS:
  --full-file               Use file-to-file strict comparison mode.
                            Compares complete .js file output (imports, class
                            definitions, static fields like ɵcmp/ɵfac/ɵdir,
                            setClassMetadata, exports) instead of just templates.
                            Requires a preset with tsconfigPath for real project
                            context. Results are at file-level, not component-level.

ANGULAR BASELINE OPTIONS:
  --generate-ng-baseline    Run Angular compilation only and save baseline to file.
                            Skips Oxc compilation and comparison. Use with
                            --save-ng-baseline to specify output path.
  --save-ng-baseline <path> Save Angular output as a baseline file after compilation.
                            Can be used with or without --generate-ng-baseline.
  --ng-baseline <path>      Load Angular output from a baseline file instead of
                            running the Angular compiler. Dramatically speeds up
                            comparison runs.

OUTPUT OPTIONS:
  --json-reporter           Output results as JSON to stdout (instead of file)
  --detailed-diff           Show detailed function body diffs for mismatches

DEBUG/VALIDATION OPTIONS:
  --validate-extraction     Enable extraction validation (compares Oxc vs TypeScript extraction)
                            Disabled by default. Can also be enabled with VALIDATE_EXTRACTION=true

EXAMPLES:
  # Compare bitwarden-clients
  pnpm compare -p ../bitwarden-clients

  # Compare with specific patterns
  pnpm compare -p ../project --include "apps/**/*.component.ts"

  # Run fixtures only
  pnpm compare --fixtures

  # Run both project and fixtures
  pnpm compare -p ../bitwarden-clients --both

  # Run specific fixture categories
  pnpm compare --fixtures --category defer --category animations

  # List all fixtures
  pnpm compare --list-fixtures

  # Use a preset for bitwarden
  pnpm compare -p ../bitwarden-clients --preset bitwarden

  # Use a preset with additional patterns
  pnpm compare -p ../angular-material --preset material-angular --include "src/cdk-experimental/**/*.ts"

  # List all presets
  pnpm compare --list-presets

  # Use file-to-file strict comparison mode with a preset
  pnpm compare --preset bitwarden --full-file

  # Generate Angular baseline (slow, run once)
  pnpm compare --preset clickup --full-file --generate-ng-baseline --save-ng-baseline ./ng-baseline-clickup.json

  # Use saved Angular baseline (fast, run many times)
  pnpm compare --preset clickup --full-file --ng-baseline ./ng-baseline-clickup.json

  # Run comparison and save baseline at the same time
  pnpm compare --preset clickup --full-file --save-ng-baseline ./ng-baseline-clickup.json

REPORT FORMAT:
  The JSON report includes:
  - summary: Statistics (total, matched, mismatched, errors, pass rate)
  - metadata: Run info (timestamp, duration, project path)
  - results: Per-component details with code output and diffs

  In --full-file mode, the report additionally includes:
  - fileComparisonMode: true
  - fileSummary: File-level statistics (files compared, matched, mismatched)
  - fileResults: Per-file comparison details with full .js output and diffs
`)
}

/**
 * The fixtures' ngtsc reference outputs, and the baseline file that saves them.
 *
 * `scripts/generate-ng-baseline.ts` compiles every fixture with ngtsc and saves the outputs, and
 * `--fixtures --ng-baseline <path>` reads them instead of compiling. This module loads only the
 * Angular side of the harness, never Oxc or the comparison.
 */

import { createHash } from 'node:crypto'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

import ts from 'typescript'

import {
  compileWithNgtsc,
  generateComponentSource,
  type NgtscMode,
} from '../src/compilers/angular-ngtsc.js'
import type { CompilerOutput } from '../src/types.js'
import type { ComponentMetadata, Fixture } from './types.js'

/**
 * Path to the tsconfig.json used for fixture compilation.
 * Using a shared tsconfig enables NgtscProgram caching across all fixtures,
 * which dramatically speeds up compilation by reusing:
 * - Parsed source files (especially @angular/*)
 * - Module resolution results
 * - The NgtscProgram for incremental compilation
 */
export const FIXTURE_TSCONFIG_PATH = join(
  dirname(fileURLToPath(import.meta.url)),
  '..',
  'tsconfig.json',
)

/**
 * The path ngtsc compiles a fixture at. Fixture paths look like `/fixtures/...`, which lies
 * outside the project root (the tsconfig directory), and ngtsc leaves `filePath` and
 * `lineNumber` out of `ɵsetClassDebugInfo` for a file outside the project, which never happens in
 * a real build. Placing the file under the project root makes ngtsc emit them.
 */
function ngtscFilePath(filePath: string): string {
  return join(dirname(FIXTURE_TSCONFIG_PATH), filePath)
}

/**
 * The ngtsc mode a fixture's reference output comes from: its `ngtscMode`, or else `full` when
 * the file has components and each is standalone and provably imports nothing, and `local`
 * otherwise.
 *
 * The file alone shows such a component has no directive dependencies, so Oxc compiles its
 * template like a full build does, with the DOM-only instructions; ngtsc's local mode assumes
 * dependencies and never uses them. Any other component needs what local mode assumes about
 * the imports it cannot see.
 */
function referenceNgtscMode(fixture: Fixture, source: string): NgtscMode {
  if (fixture.ngtscMode) {
    return fixture.ngtscMode
  }
  const file = ts.createSourceFile('fixture.ts', source, ts.ScriptTarget.Latest)
  let components = 0
  let allWithoutImports = true
  const visit = (node: ts.Node): void => {
    if (ts.isClassDeclaration(node)) {
      for (const decorator of ts.getDecorators(node) ?? []) {
        const call = decorator.expression
        if (
          ts.isCallExpression(call) &&
          ts.isIdentifier(call.expression) &&
          call.expression.text === 'Component'
        ) {
          components++
          const config = call.arguments[0]
          if (
            config === undefined ||
            !ts.isObjectLiteralExpression(config) ||
            !isStandaloneWithoutImports(config)
          ) {
            allWithoutImports = false
          }
        }
      }
    }
    ts.forEachChild(node, visit)
  }
  visit(file)
  return components > 0 && allWithoutImports ? 'full' : 'local'
}

/**
 * Whether a `@Component` config is standalone and its `imports` and `deferredImports` are each
 * absent or `[]`, with no spread or shorthand property that could hide them.
 */
function isStandaloneWithoutImports(config: ts.ObjectLiteralExpression): boolean {
  return config.properties.every((property) => {
    if (!ts.isPropertyAssignment(property)) {
      return false
    }
    const key =
      ts.isIdentifier(property.name) || ts.isStringLiteral(property.name)
        ? property.name.text
        : undefined
    if (key === 'standalone') {
      return property.initializer.kind !== ts.SyntaxKind.FalseKeyword
    }
    if (key === 'imports' || key === 'deferredImports') {
      return (
        ts.isArrayLiteralExpression(property.initializer) &&
        property.initializer.elements.length === 0
      )
    }
    return true
  })
}
/** The component source generated from a component fixture's template and metadata. */
export function componentSource(fixture: Fixture): string {
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

  return generateComponentSource(fixture.template!, fixture.className, metadata)
}

/** What ngtsc compiles for a fixture. */
type NgtscCompilation = { source: string; filePath: string; mode: NgtscMode }

/**
 * What the runner's test functions compile with ngtsc for a fixture, or `undefined` for a fixture
 * they don't. The paths must stay those the test functions compile with Oxc.
 */
export function ngtscCompilation(fixture: Fixture): NgtscCompilation | undefined {
  if (fixture.skip || fixture.skipReason) {
    return undefined
  }
  const fixtureType = fixture.type || 'component'
  if (fixtureType === 'factory' || fixtureType === 'directive' || fixtureType === 'injectable') {
    return undefined
  }
  if (fixtureType === 'full-transform' || fixtureType === 'class-metadata') {
    const source = fixture.sourceCode
    const filePath =
      fixtureType === 'full-transform'
        ? ngtscFilePath(fixture.filePath || `${fixture.category}/${fixture.name}.ts`)
        : join(dirname(FIXTURE_TSCONFIG_PATH), 'fixtures', fixture.category, `${fixture.name}.ts`)
    return source ? { source, filePath, mode: referenceNgtscMode(fixture, source) } : undefined
  }
  const source = componentSource(fixture)
  const filePath = fixture.filePath || `fixtures/${fixture.category}/${fixture.name}.component.ts`
  return { source, filePath: ngtscFilePath(filePath), mode: referenceNgtscMode(fixture, source) }
}

/**
 * The baseline key of a compilation: a hash of its mode, path and source. The path is relative to
 * the project root, except on a case-insensitive file system, where ngtsc embeds the absolute
 * path in i18n message names.
 */
export function ngBaselineKey({ source, filePath, mode }: NgtscCompilation): string {
  const path = ts.sys.useCaseSensitiveFileNames
    ? relative(dirname(FIXTURE_TSCONFIG_PATH), filePath)
    : filePath
  return createHash('sha256')
    .update(JSON.stringify([mode, path, source]))
    .digest('hex')
}

/** The ngtsc outputs read from `--ng-baseline`, by `ngBaselineKey`. */
export const ngBaseline: { outputs?: Record<string, { code: string; error?: string }> } = {}

/** A fixture's ngtsc output: from the baseline if it has it, compiled otherwise. */
export async function compileReference(fixture: Fixture): Promise<CompilerOutput> {
  const compilation = ngtscCompilation(fixture)!
  const saved = ngBaseline.outputs?.[ngBaselineKey(compilation)]
  if (saved) {
    return { ...saved, compilationTimeMs: 0 }
  }
  const { source, filePath, mode } = compilation
  return compileWithNgtsc(source, filePath, { tsconfigPath: FIXTURE_TSCONFIG_PATH, mode })
}

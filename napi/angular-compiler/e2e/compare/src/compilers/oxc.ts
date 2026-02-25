import * as path from 'path'

import {
  transformAngularFileSync,
  compileClassMetadata,
  Severity,
  type TransformOptions,
  type ResolvedResources,
} from '@oxc-angular/vite/api'
import {
  transformSync as oxcTransformSync,
  type TransformOptions as OxcTransformOptions,
} from 'oxc-transform'

import type { CompilerOutput, ProjectCompilationResult } from '../types.js'

// Re-export ResolvedResources type for use in runner
export type { ResolvedResources } from '@oxc-angular/vite/api'

// Plain object version of ResolvedResources that NAPI-RS actually expects
// (NAPI-RS HashMap bindings expect plain objects, not JavaScript Map objects)
export type PlainResolvedResources = {
  templates: Record<string, string>
  styles: Record<string, string[]>
}

/**
 * Output from raw full-file compilation.
 */
export interface OxcFullFileRawOutput {
  code: string
  error?: string
  compilationTimeMs: number
}

/**
 * Options for full-file compilation.
 */
export interface OxcFullFileRawOptions {
  /** Pre-resolved external resources (templates/styles) */
  resolvedResources?: ResolvedResources | null
  /** Path to tsconfig.json for resolving path aliases in cross-file analysis */
  tsconfigPath?: string
}

/**
 * Compile a full TypeScript file using the Oxc Angular compiler (Rust via NAPI).
 * Returns the complete compiled JavaScript output.
 *
 * @param source - Full TypeScript source code
 * @param filePath - Path to the source file
 * @param options - Compilation options
 */
export function compileWithOxcFullFileRaw(
  source: string,
  filePath: string,
  options?: OxcFullFileRawOptions,
): OxcFullFileRawOutput {
  const startTime = performance.now()

  const transformOptions: TransformOptions = {
    sourcemap: false,
    jit: false,
    hmr: false,
    advancedOptimizations: false,
    // Enable cross-file analysis for barrel export tracing
    crossFileElision: true,
    baseDir: path.dirname(filePath),
    // Pass tsconfig for resolving monorepo path aliases (e.g., @cu/*)
    tsconfigPath: options?.tsconfigPath,
    // Enable class metadata for TestBed support (matches Angular's output)
    emitClassMetadata: true,
  }

  try {
    // Step 1: Angular transformation (produces TypeScript with Angular transforms)
    const angularResult = transformAngularFileSync(
      source,
      filePath,
      transformOptions,
      options?.resolvedResources ?? null,
    )

    // Only fail on actual errors, not warnings
    const angularErrors = angularResult.errors.filter((e) => e.severity === Severity.Error)

    if (angularErrors.length > 0) {
      return {
        code: '',
        error: angularErrors.map((e) => e.message).join('\n'),
        compilationTimeMs: performance.now() - startTime,
      }
    }

    // Step 2: TypeScript to JavaScript transpilation
    // This strips type annotations and produces ES module JavaScript
    const tsTransformOptions: OxcTransformOptions = {
      lang: 'ts',
      sourceType: 'module',
      typescript: {
        // Keep all value imports. Semantic analysis would remove too many because:
        // 1. Angular transformation adds namespace refs (i1.AuthService)
        // 2. Original named imports (AuthService) appear "unused" after transformation
        // 3. But Angular keeps them because they may be used in source code
        // Setting to true matches Angular's conservative approach of keeping imports
        onlyRemoveTypeImports: true,
      },
      // Target esnext to keep modern syntax without downleveling.
      // This prevents the ES2026 explicit resource management transformer from
      // running on `using` syntax, which would inject @oxc-project/runtime imports.
      // Angular's compiler also doesn't downlevel `using` - that's handled by the bundler.
      target: 'esnext',
    }

    const jsResult = oxcTransformSync(filePath, angularResult.code, tsTransformOptions)

    // Check for transpilation errors
    const jsErrors = jsResult.errors?.filter((e) => e.severity === 'Error') ?? []
    if (jsErrors.length > 0) {
      return {
        code: '',
        error: jsErrors.map((e) => e.message).join('\n'),
        compilationTimeMs: performance.now() - startTime,
      }
    }

    return {
      code: jsResult.code,
      compilationTimeMs: performance.now() - startTime,
    }
  } catch (e) {
    return {
      code: '',
      error: e instanceof Error ? e.message : String(e),
      compilationTimeMs: performance.now() - startTime,
    }
  }
}

/**
 * Compile a full TypeScript file using the Oxc Angular compiler (Rust via NAPI).
 *
 * This uses `transformAngularFile` which processes the entire TypeScript source,
 * extracts @Component metadata, compiles templates, and generates the full output.
 *
 * Returns the complete compiled JavaScript - no extraction is performed.
 * The semantic comparison logic handles full-file comparison.
 *
 * @param source - Full TypeScript source code
 * @param filePath - Path to the source file
 * @param options - Compilation options
 */
export function compileWithOxcFullFile(
  source: string,
  filePath: string,
  options?: OxcFullFileRawOptions,
): CompilerOutput {
  const rawOutput = compileWithOxcFullFileRaw(source, filePath, options)

  if (rawOutput.error) {
    return {
      code: '',
      error: rawOutput.error,
      compilationTimeMs: rawOutput.compilationTimeMs,
    }
  }

  // Return full output - no extraction needed
  // The semantic comparison logic handles full-file comparison
  return {
    code: rawOutput.code,
    compilationTimeMs: rawOutput.compilationTimeMs,
  }
}

/**
 * Compile class metadata using the Oxc Angular compiler (Rust via NAPI).
 *
 * This compiles the setClassMetadata call for an Angular decorated class,
 * which is used by TestBed to recompile classes with overrides.
 *
 * @param source - Full TypeScript source code
 * @param filePath - Path to the source file
 * @param className - The class name to compile metadata for
 * @param decoratorType - The decorator type: "Component", "Directive", "Pipe", "Injectable", "NgModule"
 */
export async function compileClassMetadataWithOxc(
  source: string,
  filePath: string,
  className: string,
  decoratorType: string,
): Promise<CompilerOutput> {
  const startTime = performance.now()

  try {
    const result = await compileClassMetadata(source, filePath, className, decoratorType)
    const compilationTimeMs = performance.now() - startTime

    const errors = result.errors.filter((e) => e.severity === Severity.Error)

    if (errors.length > 0) {
      return {
        code: '',
        error: errors.map((e) => e.message).join('\n'),
        compilationTimeMs,
      }
    }

    return {
      code: result.code,
      compilationTimeMs,
    }
  } catch (e) {
    return {
      code: '',
      error: e instanceof Error ? e.message : String(e),
      compilationTimeMs: performance.now() - startTime,
    }
  }
}

/**
 * Options for project-wide Oxc compilation.
 */
export interface OxcProjectOptions {
  /** Pre-resolved resources (templates/styles) by file path */
  resolvedResourcesByFile?: Map<string, PlainResolvedResources>
  /** Path to tsconfig.json for resolving path aliases in cross-file analysis */
  tsconfigPath?: string
}

/**
 * Compile multiple files with Oxc sequentially.
 *
 * To avoid memory exhaustion, this function:
 * 1. Processes files ONE AT A TIME (fully synchronous, no parallelism)
 * 2. Stores results directly in Maps (no disk round-trip)
 * 3. Deletes source from fileContents after each file to free memory
 *
 * Returns results in the same format as project-wide NgtscProgram compilation.
 */
export function compileProjectWithOxc(
  filePaths: string[],
  fileContents: Map<string, string>,
  options?: OxcProjectOptions,
): ProjectCompilationResult {
  const startTime = performance.now()

  const emittedFiles = new Map<string, string>()
  const errors = new Map<string, string[]>()
  let successCount = 0
  let errorCount = 0

  // Process files sequentially
  for (let i = 0; i < filePaths.length; i++) {
    const filePath = filePaths[i]
    const source = fileContents.get(filePath)

    if (!source) {
      errors.set(filePath, ['No source content'])
      errorCount++
    } else {
      try {
        const plainResources = options?.resolvedResourcesByFile?.get(filePath)
        const compiled = compileWithOxcFullFileRaw(source, filePath, {
          resolvedResources: plainResources,
          tsconfigPath: options?.tsconfigPath,
        })

        if (compiled.error) {
          errors.set(filePath, [compiled.error])
          errorCount++
        } else {
          emittedFiles.set(filePath, compiled.code)
          successCount++
        }
      } catch (e) {
        errors.set(filePath, [e instanceof Error ? e.message : String(e)])
        errorCount++
      }

      // CRITICAL: Delete source from memory immediately after compilation
      fileContents.delete(filePath)
      options?.resolvedResourcesByFile?.delete(filePath)
    }

    // Progress update every 100 files
    if ((i + 1) % 100 === 0 || i === filePaths.length - 1) {
      process.stdout.write(`\r  Oxc: ${i + 1}/${filePaths.length} files compiled`)
    }

    // Periodic GC every 500 files
    if ((i + 1) % 500 === 0 && global.gc) {
      global.gc()
    }
  }

  process.stdout.write('\n')

  return {
    emittedFiles,
    errors,
    durationMs: performance.now() - startTime,
    successCount,
    errorCount,
  }
}

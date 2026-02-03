/**
 * NgtscProgram-based Angular compiler reference.
 *
 * This uses Angular's full AOT compilation pipeline via NgtscProgram,
 * which matches the real Angular CLI behavior more accurately than
 * using the lower-level @angular/compiler APIs directly.
 */

import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { NgtscProgram, EmitFlags, type CompilerOptions } from '@angular/compiler-cli'
import ts from 'typescript'

import type { ComponentMetadata } from '../../fixtures/types.js'
import type { CompilerOutput, ProjectCompilationResult } from '../types.js'

// Get the compare tool's node_modules path for hybrid module resolution
const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const COMPARE_TOOL_NODE_MODULES = path.resolve(__dirname, '../../node_modules')

// =============================================================================
// Program Cache Implementation
// =============================================================================

/**
 * Cached program entry with metadata for cache management.
 */
interface CachedProgramEntry {
  /** The compiler options used to create this program */
  mergedOptions: CompilerOptions
  /** The compiler host used by this program */
  host: ts.CompilerHost
  /** The project root directory */
  projectRoot: string
  /** Hash of the tsconfig content for invalidation */
  tsconfigHash: string
  /** Timestamp when this entry was created */
  createdAt: number
  /** Last access time for LRU eviction */
  lastAccessedAt: number
  /**
   * The last NgtscProgram created for this project.
   * Used as oldProgram for incremental compilation - enables TypeScript
   * to reuse type-checked source files and Angular to reuse analysis cache.
   */
  lastProgram?: NgtscProgram
}

/**
 * Cache for TypeScript compiler configurations.
 * Keyed by tsconfig path (or a synthetic key for virtual mode).
 */
const programConfigCache = new Map<string, CachedProgramEntry>()

/**
 * Special cache key for virtual mode compilations.
 * Virtual mode doesn't have a tsconfig, so we use a fixed key.
 */
const VIRTUAL_MODE_CACHE_KEY = '__virtual_mode__'

/**
 * Cached virtual mode configuration.
 * Reused across all virtual mode compilations to avoid re-creating
 * the host and compiler options for each file.
 */
let virtualModeConfig: CachedProgramEntry | null = null

/** Maximum age for cache entries (5 minutes) */
const CACHE_MAX_AGE_MS = 5 * 60 * 1000

/** Maximum number of cached entries */
const CACHE_MAX_SIZE = 10

/**
 * Simple hash function for string content.
 * Uses djb2 algorithm for reasonable distribution.
 */
function hashString(str: string): string {
  let hash = 5381
  for (let i = 0; i < str.length; i++) {
    const char = str.charCodeAt(i)
    hash = ((hash << 5) + hash) ^ char // hash * 33 ^ char
  }
  // Convert to unsigned 32-bit integer, then to hex string
  return (hash >>> 0).toString(16)
}

// =============================================================================
// Diagnostic Warning Tracking
// =============================================================================

/**
 * Track diagnostic collection failures to batch warnings.
 * These warnings don't affect compilation output - code is still emitted successfully.
 */
const diagnosticWarnings = new Set<string>()

/**
 * Print a summary of diagnostic collection warnings if any occurred.
 * Call this after all compilations are complete.
 */
export function printDiagnosticWarningSummary(): void {
  if (diagnosticWarnings.size > 0) {
    console.warn(
      `\nNote: Could not collect diagnostics for ${diagnosticWarnings.size} file(s) due to unresolved imports in virtual mode.`,
    )
    console.warn(`This does not affect compilation output - all code was emitted successfully.\n`)
    diagnosticWarnings.clear()
  }
}

/**
 * Clear diagnostic warning tracking (for test isolation).
 */
export function clearDiagnosticWarnings(): void {
  diagnosticWarnings.clear()
}

// =============================================================================
// Diagnostic Cache Implementation
// =============================================================================

/**
 * Cached diagnostic entry.
 * Diagnostics are purely informational and don't affect compilation output,
 * so they can be safely cached based on source content.
 */
interface CachedDiagnosticEntry {
  /** The cached diagnostics (already filtered for errors) */
  diagnostics: ts.Diagnostic[]
  /** Hash of the source content for validation */
  sourceHash: string
}

/**
 * Cache for diagnostics.
 * Keyed by file path + source hash to ensure uniqueness.
 */
const diagnosticCache = new Map<string, CachedDiagnosticEntry>()

/** Maximum number of cached diagnostic entries */
const DIAGNOSTIC_CACHE_MAX_SIZE = 100

// =============================================================================
// Global Shared Caches for Performance
// =============================================================================

/**
 * Global shared module resolution cache for performance.
 * Reused across all compilation calls to avoid redundant module lookups.
 */
let sharedModuleResolutionCache: ts.ModuleResolutionCache | null = null

/**
 * Get or create the shared module resolution cache.
 * Using a shared cache dramatically speeds up module resolution across files.
 */
function getOrCreateModuleResolutionCache(options: ts.CompilerOptions): ts.ModuleResolutionCache {
  if (!sharedModuleResolutionCache) {
    sharedModuleResolutionCache = ts.createModuleResolutionCache(
      ts.sys.getCurrentDirectory(),
      (fileName) => (ts.sys.useCaseSensitiveFileNames ? fileName : fileName.toLowerCase()),
      options,
    )
  }
  return sharedModuleResolutionCache
}

/**
 * Clear the shared module resolution cache.
 * Call this when compiler options change or when starting a new batch.
 */
export function clearModuleResolutionCache(): void {
  sharedModuleResolutionCache = null
}

/**
 * Global source file cache for performance.
 * Caches parsed source files to avoid redundant parsing.
 */
const globalSourceFileCache = new Map<string, ts.SourceFile>()

/**
 * Augment a compiler host with source file caching.
 * This prevents redundant parsing of the same files across compilations.
 */
function augmentHostWithSourceFileCaching(host: ts.CompilerHost): ts.CompilerHost {
  // oxlint-disable-next-line unbound-method
  const baseGetSourceFile = host.getSourceFile
  return {
    ...host,
    getSourceFile: (
      fileName: string,
      languageVersion: ts.ScriptTarget,
      onError?: (message: string) => void,
      shouldCreateNewSourceFile?: boolean,
    ): ts.SourceFile | undefined => {
      // If not forced to create new and we have it cached, return cached version
      if (!shouldCreateNewSourceFile && globalSourceFileCache.has(fileName)) {
        return globalSourceFileCache.get(fileName)
      }
      // Call base implementation
      const file = baseGetSourceFile.call(host, fileName, languageVersion, onError, true)
      if (file) {
        globalSourceFileCache.set(fileName, file)
      }
      return file
    },
  }
}

/**
 * Clear the global source file cache.
 * Call this when files on disk may have changed.
 */
export function clearSourceFileCache(): void {
  globalSourceFileCache.clear()
}

/**
 * Clean up diagnostic cache if it grows too large.
 * Keeps only the most recent entries.
 */
function cleanupDiagnosticCache(): void {
  if (diagnosticCache.size > DIAGNOSTIC_CACHE_MAX_SIZE) {
    // Keep only the most recent 50 entries
    const entries = [...diagnosticCache.entries()]
    diagnosticCache.clear()
    for (const [key, value] of entries.slice(-50)) {
      diagnosticCache.set(key, value)
    }
  }
}

/**
 * Clear the diagnostic cache.
 * Useful for testing or when compiler options change.
 */
export function clearDiagnosticCache(): void {
  diagnosticCache.clear()
}

/**
 * Get the current diagnostic cache size (for debugging/monitoring).
 */
export function getDiagnosticCacheSize(): number {
  return diagnosticCache.size
}

/**
 * Clean up old cache entries.
 * Removes entries older than CACHE_MAX_AGE_MS.
 */
function cleanupCache(): void {
  const now = Date.now()
  for (const [key, entry] of programConfigCache) {
    if (now - entry.createdAt > CACHE_MAX_AGE_MS) {
      programConfigCache.delete(key)
    }
  }
}

/**
 * Evict least recently used entries if cache is full.
 */
function evictIfNeeded(): void {
  if (programConfigCache.size >= CACHE_MAX_SIZE) {
    // Find the least recently accessed entry
    let oldestKey: string | null = null
    let oldestTime = Infinity

    for (const [key, entry] of programConfigCache) {
      if (entry.lastAccessedAt < oldestTime) {
        oldestTime = entry.lastAccessedAt
        oldestKey = key
      }
    }

    if (oldestKey) {
      programConfigCache.delete(oldestKey)
    }
  }
}

/**
 * Clear the entire program cache.
 * Useful for testing or when the project structure changes significantly.
 */
export function clearProgramCache(): void {
  programConfigCache.clear()
}

/**
 * Get the current cache size (for debugging/monitoring).
 */
export function getProgramCacheSize(): number {
  return programConfigCache.size
}

/**
 * Get cache statistics for debugging/monitoring.
 * Returns information about cached entries including whether they have
 * a lastProgram for incremental compilation.
 */
export function getProgramCacheStats(): {
  size: number
  entries: Array<{
    key: string
    hasLastProgram: boolean
    ageMs: number
  }>
} {
  const now = Date.now()
  const entries: Array<{ key: string; hasLastProgram: boolean; ageMs: number }> = []

  for (const [key, entry] of programConfigCache) {
    entries.push({
      key,
      hasLastProgram: entry.lastProgram !== undefined,
      ageMs: now - entry.createdAt,
    })
  }

  return {
    size: programConfigCache.size,
    entries,
  }
}

/**
 * Create a compiler host with hybrid module resolution.
 *
 * This resolves modules in a specific order:
 * 1. @angular/* imports - Resolve from the compare tool's node_modules
 *    (allows compilation without target project having node_modules installed)
 * 2. All other imports - Use standard TypeScript resolution with tsconfig paths
 *
 * This enables compiling files from external projects (like bitwarden-clients)
 * without requiring them to have node_modules installed, while still respecting
 * their tsconfig path mappings for project-internal imports.
 *
 * @param baseHost - The base compiler host to delegate to
 * @param compilerOptions - Compiler options (includes paths from tsconfig)
 * @returns A compiler host with hybrid module resolution
 */
function createHybridResolveHost(
  baseHost: ts.CompilerHost,
  compilerOptions: ts.CompilerOptions,
): ts.CompilerHost {
  // Use shared module resolution cache for better performance across files
  const moduleResolutionCache = getOrCreateModuleResolutionCache(compilerOptions)

  // Create a "fake" containing file path in the compare tool's node_modules
  // for resolving @angular/* packages
  const angularContainingFile = path.join(COMPARE_TOOL_NODE_MODULES, '__resolve__.ts')

  return {
    ...baseHost,
    resolveModuleNames: (
      moduleNames: string[],
      containingFile: string,
      _reusedNames: string[] | undefined,
      _redirectedReference: ts.ResolvedProjectReference | undefined,
      options: ts.CompilerOptions,
    ): (ts.ResolvedModule | undefined)[] => {
      return moduleNames.map((moduleName) => {
        // 1. @angular/* packages - resolve from compare tool's node_modules
        // This allows compilation without target project having @angular installed
        if (moduleName.startsWith('@angular/')) {
          const resolved = ts.resolveModuleName(
            moduleName,
            angularContainingFile,
            options,
            baseHost,
            moduleResolutionCache,
          )
          return resolved.resolvedModule
        }

        // 2. rxjs and zone.js - also resolve from compare tool's node_modules
        // These are common Angular dependencies
        if (moduleName.startsWith('rxjs') || moduleName === 'zone.js') {
          const resolved = ts.resolveModuleName(
            moduleName,
            angularContainingFile,
            options,
            baseHost,
            moduleResolutionCache,
          )
          return resolved.resolvedModule
        }

        // 3. Standard resolution for everything else
        // This uses tsconfig paths for project aliases like @bitwarden/*
        const resolved = ts.resolveModuleName(
          moduleName,
          containingFile,
          options,
          baseHost,
          moduleResolutionCache,
        )
        return resolved.resolvedModule
      })
    },
  }
}

/**
 * Get or create a cached program configuration for a tsconfig path.
 * This caches the expensive tsconfig parsing and host creation,
 * but creates a new NgtscProgram for each file (since NgtscProgram
 * doesn't support incrementally adding files).
 *
 * @param tsconfigPath - Path to the tsconfig.json file
 * @returns Cached or newly created program configuration
 */
function getOrCreateCachedConfig(tsconfigPath: string): CachedProgramEntry {
  // Run cleanup periodically
  cleanupCache()

  // Check if we have a valid cached entry
  const cached = programConfigCache.get(tsconfigPath)
  if (cached) {
    // Verify the tsconfig hasn't changed by checking the hash
    const currentRawConfig = ts.sys.readFile(tsconfigPath) ?? ''
    const currentHash = hashString(currentRawConfig)

    if (cached.tsconfigHash === currentHash) {
      // Cache hit - update access time and return
      cached.lastAccessedAt = Date.now()
      return cached
    }

    // Tsconfig changed - invalidate this entry
    programConfigCache.delete(tsconfigPath)
  }

  // Evict if needed before adding new entry
  evictIfNeeded()

  // Load and parse tsconfig
  const { compilerOptions, projectRoot, rawConfig } = loadTsConfig(tsconfigPath)
  const tsconfigHash = hashString(rawConfig)

  // Merge with Angular options first (needed for host creation)
  const angularOptions = createAngularOptions()
  const mergedOptions = {
    ...compilerOptions,
    ...angularOptions,
    noEmitOnError: false,
    skipLibCheck: true,
    // Force ESNext target to eliminate ES transform differences (__awaiter, etc.)
    target: ts.ScriptTarget.ESNext,
    // Disable decorator metadata and experimental decorators to prevent __decorate emission
    // Angular's own decorators are handled by ngtsc, custom decorators are left as-is
    emitDecoratorMetadata: false,
    experimentalDecorators: false,
    // Disable importHelpers to prevent tslib imports for decorator helpers
    importHelpers: false,
  } as CompilerOptions

  // Create a base compiler host
  const baseHost = ts.createCompilerHost(mergedOptions)

  // Wrap with hybrid module resolution to resolve @angular/* from compare tool's node_modules
  // while still using project's tsconfig paths for internal aliases
  const hybridHost = createHybridResolveHost(baseHost, mergedOptions)

  const entry: CachedProgramEntry = {
    mergedOptions,
    host: hybridHost,
    projectRoot,
    tsconfigHash,
    createdAt: Date.now(),
    lastAccessedAt: Date.now(),
  }

  programConfigCache.set(tsconfigPath, entry)
  return entry
}

/**
 * Create a file-specific compiler host that overrides source file access
 * for a specific target file while delegating to the base host for everything else.
 *
 * @param baseHost - The base compiler host to delegate to
 * @param targetPath - The absolute path of the target file
 * @param source - The source content to inject for the target file
 * @returns A new compiler host with the file override
 */
function createFileOverrideHost(
  baseHost: ts.CompilerHost,
  targetPath: string,
  source: string,
): ts.CompilerHost {
  const normalizedTarget = path.normalize(targetPath)

  return {
    ...baseHost,
    getSourceFile: (
      fileName: string,
      languageVersion: ts.ScriptTarget,
      onError?: (message: string) => void,
    ): ts.SourceFile | undefined => {
      if (path.normalize(fileName) === normalizedTarget) {
        return ts.createSourceFile(fileName, source, languageVersion, true)
      }
      return baseHost.getSourceFile(fileName, languageVersion, onError)
    },
    fileExists: (fileName: string): boolean => {
      if (path.normalize(fileName) === normalizedTarget) {
        return true
      }
      return baseHost.fileExists(fileName)
    },
    readFile: (fileName: string): string | undefined => {
      if (path.normalize(fileName) === normalizedTarget) {
        return source
      }
      return baseHost.readFile(fileName)
    },
  }
}

/**
 * Options for NgtscProgram compilation.
 */
export interface NgtscOptions {
  /** Path to tsconfig.json for real project context */
  tsconfigPath?: string
}

/**
 * Create minimal tsconfig options for Angular compilation.
 */
function createCompilerOptions(rootDir: string): ts.CompilerOptions {
  return {
    // Use ESNext to preserve decorators as-is (no __esDecorate helper)
    target: ts.ScriptTarget.ESNext,
    module: ts.ModuleKind.ESNext,
    // Use Bundler module resolution to support package subpath exports
    // (e.g., @angular/common/http which uses exports field in package.json)
    moduleResolution: ts.ModuleResolutionKind.Bundler,
    lib: ['lib.es2022.d.ts', 'lib.dom.d.ts'],
    declaration: false,
    emitDecoratorMetadata: false,
    experimentalDecorators: false,
    importHelpers: false,
    strict: true,
    noEmitOnError: false,
    skipLibCheck: true,
    esModuleInterop: true,
    allowSyntheticDefaultImports: true,
    resolveJsonModule: true,
    isolatedModules: true,
    noImplicitAny: false,
    strictNullChecks: false,
    strictPropertyInitialization: false,
    baseUrl: rootDir,
    rootDir,
    outDir: path.join(rootDir, 'dist'),
    // Angular-specific options
    // These match Angular CLI's default settings
  }
}

/**
 * Create Angular compiler options.
 * @param skipTypeChecking - If true, disable expensive type checking (for batch/comparison mode)
 */
function createAngularOptions(skipTypeChecking = false): Record<string, unknown> {
  if (skipTypeChecking) {
    // Skip all expensive type checking - we only need the emitted template code
    // Use 'experimental-local' mode for fast compilation (1.6s vs 23+ min for full mode)
    // Note: experimental-local produces slightly different output than full mode
    return {
      strictTemplates: false, // Skip template type checking
      strictInjectionParameters: false,
      strictInputAccessModifiers: false,
      enableI18nLegacyMessageIdFormat: false,
      compilationMode: 'experimental-local', // Fast mode for comparison
      // Additional options to skip expensive checks
      extendedDiagnostics: undefined, // Skip extended diagnostics
    }
  }
  return {
    strictTemplates: true,
    strictInjectionParameters: true,
    strictInputAccessModifiers: true,
    enableI18nLegacyMessageIdFormat: false,
    compilationMode: 'full',
  }
}

/**
 * Load tsconfig and parse its contents.
 * Returns the parsed compiler options, resolved path mappings, and raw config for hashing.
 */
export function loadTsConfig(tsconfigPath: string): {
  compilerOptions: ts.CompilerOptions
  projectRoot: string
  rawConfig: string
} {
  const rawConfig = ts.sys.readFile(tsconfigPath) ?? ''
  // oxlint-disable-next-line unbound-method
  const configFile = ts.readConfigFile(tsconfigPath, ts.sys.readFile)
  if (configFile.error) {
    throw new Error(
      `Failed to read tsconfig: ${ts.flattenDiagnosticMessageText(configFile.error.messageText, '\n')}`,
    )
  }

  const projectRoot = path.dirname(tsconfigPath)
  const parsed = ts.parseJsonConfigFileContent(
    configFile.config,
    ts.sys,
    projectRoot,
    undefined,
    tsconfigPath,
  )

  if (parsed.errors.length > 0) {
    const errors = parsed.errors
      .map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'))
      .join('\n')
    throw new Error(`Failed to parse tsconfig: ${errors}`)
  }

  return {
    compilerOptions: parsed.options,
    projectRoot,
    rawConfig,
  }
}

/**
 * Compile a full TypeScript file using Angular's NgtscProgram.
 *
 * This uses the full AOT compilation pipeline which:
 * - Analyzes component metadata from decorators
 * - Compiles templates
 * - Generates factory functions
 * - Emits the transformed JavaScript
 *
 * Returns the COMPLETE emitted JavaScript without any extraction.
 * The comparison logic (compareFullFileSemantically) handles full-file semantic comparison.
 *
 * This function uses caching to avoid re-parsing tsconfig and re-creating compiler
 * hosts for each file when compiling multiple files from the same project.
 *
 * @param source - Full TypeScript source code
 * @param filePath - Path to the source file (used for error messages and module resolution)
 * @param options - Optional compilation options (tsconfigPath for real project context)
 */
export async function compileWithNgtsc(
  source: string,
  filePath: string,
  options?: NgtscOptions,
): Promise<CompilerOutput> {
  const startTime = performance.now()

  let absolutePath: string
  let host: ts.CompilerHost
  let mergedOptions: CompilerOptions
  let cachedConfig: CachedProgramEntry | undefined

  if (options?.tsconfigPath) {
    // Real project mode: use cached tsconfig and host
    cachedConfig = getOrCreateCachedConfig(options.tsconfigPath)

    // Use the actual file path directly (it should already be absolute)
    absolutePath = path.isAbsolute(filePath)
      ? filePath
      : path.resolve(cachedConfig.projectRoot, filePath)

    // Create a file-specific host that overrides source file access for our target
    host = createFileOverrideHost(cachedConfig.host, absolutePath, source)
    mergedOptions = cachedConfig.mergedOptions
  } else {
    // Virtual filesystem mode: use cached config for performance
    // All virtual mode compilations share the same compiler options and base host
    const compareToolDir = path.resolve(path.dirname(new URL(import.meta.url).pathname), '../..')

    // Create a virtual file path within the compare tool directory
    // This ensures @angular/core and other modules can be resolved from node_modules
    const virtualFileName = path.basename(filePath)
    absolutePath = path.join(compareToolDir, 'virtual', virtualFileName)
    const rootDir = compareToolDir

    // Get or create the shared virtual mode config
    if (!virtualModeConfig) {
      // Create compiler options (only once)
      const compilerOptions = createCompilerOptions(rootDir)
      const angularOptions = createAngularOptions()
      const mergedCompilerOptions = {
        ...compilerOptions,
        ...angularOptions,
      } as CompilerOptions

      // Create a base host with hybrid module resolution
      const baseHost = ts.createCompilerHost(mergedCompilerOptions)
      const hybridHost = createHybridResolveHost(baseHost, mergedCompilerOptions)

      // Apply source file caching to speed up parsing of @angular/* modules
      const cachedHost = augmentHostWithSourceFileCaching(hybridHost)

      virtualModeConfig = {
        mergedOptions: mergedCompilerOptions,
        host: cachedHost,
        projectRoot: rootDir,
        tsconfigHash: 'virtual',
        createdAt: Date.now(),
        lastAccessedAt: Date.now(),
      }

      // Also add to program cache for visibility in stats
      programConfigCache.set(VIRTUAL_MODE_CACHE_KEY, virtualModeConfig)
    }

    // Update access time
    virtualModeConfig.lastAccessedAt = Date.now()
    cachedConfig = virtualModeConfig
    mergedOptions = virtualModeConfig.mergedOptions

    // Create virtual files map with the source
    const virtualFiles = new Map<string, string>()
    virtualFiles.set(absolutePath, source)

    // Create a file-override host that wraps the cached host
    // This allows the virtual file to be read while reusing cached @angular/* source files
    host = createFileOverrideHost(virtualModeConfig.host, absolutePath, source)
  }

  // Create NgtscProgram with root names, options, and host
  // NgtscProgram constructor: (rootNames, options, delegateHost, oldProgram?)
  // Pass the previous program as oldProgram to enable incremental compilation:
  // - TypeScript can reuse type-checked source files from the previous program
  // - Angular can reuse its analysis cache for unchanged files
  const oldProgram = cachedConfig?.lastProgram
  const ngProgram = new NgtscProgram(
    [absolutePath], // rootNames
    mergedOptions, // options
    host, // delegateHost (CompilerHost)
    oldProgram, // oldProgram for incremental compilation
  )

  // Update the cached program for future incremental compilations
  if (cachedConfig) {
    cachedConfig.lastProgram = ngProgram
  }

  // Get the Angular compiler instance
  const ngCompiler = ngProgram.compiler

  // Run analysis phase
  await ngCompiler.analyzeAsync()

  // Collect emitted output
  let emittedCode = ''

  // Create a custom write file callback that captures output
  const writeFileCallback: ts.WriteFileCallback = (fileName: string, data: string) => {
    // Only capture .js output (not .d.ts or .map)
    if (fileName.endsWith('.js')) {
      emittedCode = data
    }
  }

  // Emit using Angular's transformers
  // Use emitCallback to provide writeFile through TsEmitArguments
  const emitResult = ngProgram.emit({
    emitFlags: EmitFlags.Default,
    forceEmit: true,
    emitCallback: (args) => {
      return args.program.emit(
        args.targetSourceFile,
        writeFileCallback,
        args.cancellationToken,
        false, // emitOnlyDtsFiles
        args.customTransformers,
      )
    },
  })

  const compilationTimeMs = performance.now() - startTime

  // Check for errors using diagnostic cache
  // Diagnostics are informational and don't affect compilation output,
  // so we can safely cache them based on source content.
  const sourceHash = hashString(source)
  const diagnosticCacheKey = `${filePath}:${sourceHash}`
  const cachedDiagnostic = diagnosticCache.get(diagnosticCacheKey)

  let diagnostics: ts.Diagnostic[] = []

  if (cachedDiagnostic && cachedDiagnostic.sourceHash === sourceHash) {
    // Use cached diagnostics
    diagnostics = cachedDiagnostic.diagnostics
  } else {
    // Collect diagnostics
    // Wrap in try-catch because getTsSemanticDiagnostics can fail when:
    // - A component imports from other project files that don't exist in our virtual filesystem
    // - TypeScript tries to get diagnostic locations for missing files and crashes
    //   with "Cannot destructure property 'pos' of 'file.referencedFiles[index]' as it is undefined"
    try {
      diagnostics = [
        ...ngProgram.getTsSyntacticDiagnostics(),
        ...ngProgram.getTsSemanticDiagnostics(),
        ...ngCompiler.getDiagnostics(),
      ]

      // Cache the diagnostics for future compilations
      cleanupDiagnosticCache()
      diagnosticCache.set(diagnosticCacheKey, {
        diagnostics,
        sourceHash,
      })
    } catch (diagnosticError) {
      // If we can't get diagnostics but have emitted code, continue silently
      // This happens when compiling components that import from other project files
      if (emittedCode) {
        // Track the warning for later summary instead of printing each one
        diagnosticWarnings.add(filePath)
      } else {
        // No emitted code and can't get diagnostics - return meaningful error
        return {
          code: '',
          error: `Compilation failed - unable to collect diagnostics (likely due to unresolved imports to project files): ${diagnosticError instanceof Error ? diagnosticError.message : String(diagnosticError)}`,
          compilationTimeMs,
        }
      }
    }
  }

  // Check if we have emitted code - if so, return it regardless of type errors
  // With noEmitOnError: false and forceEmit: true, Angular emits code even with type errors
  if (emittedCode) {
    return {
      code: emittedCode,
      compilationTimeMs,
    }
  }

  // No emitted code - check why
  if (emitResult.emitSkipped) {
    return {
      code: '',
      error: 'Emit was skipped',
      compilationTimeMs,
    }
  }

  // Check for errors only if we didn't get emitted code
  const errors = diagnostics.filter((d) => d.category === ts.DiagnosticCategory.Error)
  if (errors.length > 0) {
    const errorMessages = errors
      .map((d) => ts.flattenDiagnosticMessageText(d.messageText, '\n'))
      .join('\n')

    return {
      code: '',
      error: errorMessages,
      compilationTimeMs,
    }
  }

  // No errors but no code either - shouldn't happen
  return {
    code: '',
    error: 'No output generated',
    compilationTimeMs,
  }
}

// =============================================================================
// Project-wide Compilation (Single NgtscProgram)
// =============================================================================

/**
 * Create a multi-file override host that overrides source file access for
 * MULTIPLE files while delegating to the base host for everything else.
 *
 * Performance optimizations:
 * - Pre-normalizes all paths once at initialization (not on every call)
 * - Caches created source files to avoid redundant parsing
 *
 * @param baseHost - The base compiler host to delegate to
 * @param fileContents - Map of absolute file path -> source content
 * @returns A new compiler host with the file overrides
 */
function createMultiFileOverrideHost(
  baseHost: ts.CompilerHost,
  fileContents: Map<string, string>,
): ts.CompilerHost {
  // Pre-normalize all paths once at initialization (not on every call)
  const normalizedContents = new Map<string, string>()
  for (const [filePath, content] of fileContents) {
    normalizedContents.set(path.normalize(filePath), content)
  }

  // Cache created source files to avoid redundant parsing
  const sourceFileCache = new Map<string, ts.SourceFile>()

  return {
    ...baseHost,
    getSourceFile: (
      fileName: string,
      languageVersion: ts.ScriptTarget,
      onError?: (message: string) => void,
    ): ts.SourceFile | undefined => {
      const normalized = path.normalize(fileName)

      // Check cache first
      const cached = sourceFileCache.get(normalized)
      if (cached) {
        return cached
      }

      const content = normalizedContents.get(normalized)
      if (content !== undefined) {
        const sf = ts.createSourceFile(fileName, content, languageVersion, true)
        sourceFileCache.set(normalized, sf)
        return sf
      }
      return baseHost.getSourceFile(fileName, languageVersion, onError)
    },
    fileExists: (fileName: string): boolean => {
      const normalized = path.normalize(fileName)
      if (normalizedContents.has(normalized)) {
        return true
      }
      // For CSS files, pretend they exist even if they don't.
      // Angular's Material library uses SCSS files but references .css in styleUrl.
      // The actual .css files are generated at build time by Angular CLI/webpack.
      // For AOT compilation comparison, we only care about templates, not styles.
      if (normalized.endsWith('.css')) {
        return true
      }
      return baseHost.fileExists(fileName)
    },
    readFile: (fileName: string): string | undefined => {
      const normalized = path.normalize(fileName)
      if (normalizedContents.has(normalized)) {
        return normalizedContents.get(normalized)
      }
      // For CSS files that don't exist on disk, return empty string.
      // This allows Angular AOT compilation to proceed without actual stylesheets.
      // We only need templates for compilation comparison.
      if (normalized.endsWith('.css')) {
        const realContent = baseHost.readFile(fileName)
        if (realContent === undefined) {
          return '/* empty */'
        }
        return realContent
      }
      return baseHost.readFile(fileName)
    },
  }
}

/**
 * Calculate the common root directory for a set of source files.
 * This mimics TypeScript's behavior when rootDir is not explicitly specified.
 *
 * @param sourceFiles - Array of absolute source file paths
 * @returns The common root directory
 */
function computeCommonRoot(sourceFiles: string[]): string {
  if (sourceFiles.length === 0) return ''
  if (sourceFiles.length === 1) return path.dirname(sourceFiles[0])

  // Find common prefix of all paths
  const parts = sourceFiles.map((f) => f.split(path.sep))
  const minLength = Math.min(...parts.map((p) => p.length))

  const commonParts: string[] = []
  for (let i = 0; i < minLength; i++) {
    const part = parts[0][i]
    if (parts.every((p) => p[i] === part)) {
      commonParts.push(part)
    } else {
      break
    }
  }

  return commonParts.join(path.sep)
}

/**
 * Build a mapping from expected output paths to source paths.
 * This correctly handles TypeScript's rootDir calculation and path transformations.
 *
 * @param sourceFiles - Array of absolute source file paths
 * @param options - Compiler options containing outDir and rootDir
 * @returns Map from normalized output path to source path
 */
function buildOutputToSourceMap(
  sourceFiles: string[],
  options: ts.CompilerOptions,
): Map<string, string> {
  const outDir = options.outDir || ''
  const rootDir = options.rootDir || computeCommonRoot(sourceFiles)

  const outputToSource = new Map<string, string>()

  for (const sourceFile of sourceFiles) {
    // Calculate relative path from rootDir
    let relativePath: string
    if (rootDir && sourceFile.startsWith(rootDir)) {
      relativePath = sourceFile.slice(rootDir.length)
      // Remove leading separator
      if (relativePath.startsWith(path.sep)) {
        relativePath = relativePath.slice(1)
      }
    } else {
      // Fallback: use basename
      relativePath = path.basename(sourceFile)
    }

    // Calculate expected output path
    const outputPath = path.join(outDir, relativePath.replace(/\.ts$/, '.js'))
    const normalizedOutput = path.normalize(outputPath)

    outputToSource.set(normalizedOutput, sourceFile)
  }

  return outputToSource
}

/**
 * Compile an entire project with parallel NgtscProgram instances.
 *
 * This splits files into chunks and compiles each chunk with a separate
 * NgtscProgram in parallel. This is faster than a single program for large
 * file sets because:
 * - analyzeAsync() scales poorly with many files (O(n²) in some cases)
 * - Multiple smaller programs complete faster in parallel
 *
 * Performance optimizations:
 * - Splits files into 8-16 chunks for parallel processing
 * - Disables strict template type checking (we only need emitted code)
 * - Skips TypeScript lib checking
 * - Skips diagnostics collection
 *
 * Use this for full-file mode batch compilation.
 *
 * @param filePaths - Array of absolute file paths to compile
 * @param tsconfigPath - Path to tsconfig.json
 * @param fileContents - Optional map of file path -> content (for virtual files)
 * @returns Project compilation result with all emitted files and errors
 */
export async function compileProjectWithNgtsc(
  filePaths: string[],
  tsconfigPath: string,
  fileContents?: Map<string, string>,
): Promise<ProjectCompilationResult> {
  const startTime = performance.now()
  const emittedFiles = new Map<string, string>()
  const fileErrors = new Map<string, string[]>()

  // Load tsconfig
  const { compilerOptions, projectRoot } = loadTsConfig(tsconfigPath)

  // Merge with Angular options - skip type checking for speed
  // We only need the emitted template code, not type errors
  const angularOptions = createAngularOptions(true) // skipTypeChecking = true
  const mergedOptions = {
    ...compilerOptions,
    ...angularOptions,
    // Use experimental-local mode for fast compilation
    compilationMode: 'experimental-local',
    noEmitOnError: false,
    skipLibCheck: true,
    // Additional TypeScript optimizations for speed
    skipDefaultLibCheck: true,
    // Don't check unused locals/parameters (faster)
    noUnusedLocals: false,
    noUnusedParameters: false,
    // Skip strict checks (we only need emitted code)
    strict: false,
    noImplicitAny: false,
    strictNullChecks: false,
    strictFunctionTypes: false,
    strictBindCallApply: false,
    strictPropertyInitialization: false,
    // Force ESNext target to eliminate ES transform differences (__awaiter, etc.)
    target: ts.ScriptTarget.ESNext,
    // Disable decorator metadata and experimental decorators to prevent __decorate emission
    emitDecoratorMetadata: false,
    experimentalDecorators: false,
    // Disable importHelpers to prevent tslib imports for decorator helpers
    importHelpers: false,
  } as CompilerOptions

  // Normalize file paths
  const absolutePaths = filePaths.map((fp) =>
    path.isAbsolute(fp) ? fp : path.resolve(projectRoot, fp),
  )

  // Split files into chunks for parallel compilation
  // analyzeAsync() scales poorly with many files, so smaller chunks in parallel are faster
  const NUM_CHUNKS = 16 // Good balance of parallelism and per-chunk overhead
  const chunkSize = Math.ceil(absolutePaths.length / NUM_CHUNKS)
  const chunks: string[][] = []
  for (let i = 0; i < absolutePaths.length; i += chunkSize) {
    chunks.push(absolutePaths.slice(i, i + chunkSize))
  }

  // Compile each chunk in parallel
  const chunkResults = await Promise.all(
    chunks.map(async (chunkPaths) => {
      const chunkEmittedFiles = new Map<string, string>()
      const chunkErrors = new Map<string, string[]>()

      try {
        // Create base host with hybrid module resolution for this chunk
        const baseHost = ts.createCompilerHost(mergedOptions)
        const hybridHost = createHybridResolveHost(baseHost, mergedOptions)

        // Apply source file caching to speed up parsing
        const cachedHost = augmentHostWithSourceFileCaching(hybridHost)

        // Create host with file overrides if provided
        let host: ts.CompilerHost
        if (fileContents && fileContents.size > 0) {
          // Filter fileContents to only include this chunk's files for efficiency
          const chunkContents = new Map<string, string>()
          for (const fp of chunkPaths) {
            const content = fileContents.get(fp)
            if (content) {
              chunkContents.set(fp, content)
            }
          }
          host = createMultiFileOverrideHost(cachedHost, chunkContents)
        } else {
          host = cachedHost
        }

        // Build a mapping from expected output paths to source paths for this chunk
        const outputToSource = buildOutputToSourceMap(chunkPaths, mergedOptions)

        // Create NgtscProgram for this chunk only
        const ngProgram = new NgtscProgram(chunkPaths, mergedOptions, host, undefined)

        // Run analysis phase for this chunk
        await ngProgram.compiler.analyzeAsync()

        // Emit this chunk's files
        ngProgram.emit({
          emitFlags: EmitFlags.Default,
          forceEmit: true,
          emitCallback: (args) => {
            return args.program.emit(
              undefined,
              (fileName, data) => {
                if (fileName.endsWith('.js')) {
                  // Look up the source file for this output path
                  const normalizedOutput = path.normalize(fileName)
                  const inputPath = outputToSource.get(normalizedOutput)
                  if (inputPath) {
                    chunkEmittedFiles.set(inputPath, data)
                  }
                }
              },
              args.cancellationToken,
              false,
              args.customTransformers,
            )
          },
        })
      } catch (e) {
        // Record error for all files in this chunk
        const errorMsg = e instanceof Error ? e.message : String(e)
        for (const fp of chunkPaths) {
          chunkErrors.set(fp, [errorMsg])
        }
      }

      return { emittedFiles: chunkEmittedFiles, errors: chunkErrors }
    }),
  )

  // Merge chunk results
  for (const result of chunkResults) {
    for (const [fp, code] of result.emittedFiles) {
      emittedFiles.set(fp, code)
    }
    for (const [fp, errors] of result.errors) {
      fileErrors.set(fp, errors)
    }
  }

  const durationMs = performance.now() - startTime

  return {
    emittedFiles,
    errors: fileErrors,
    durationMs,
    successCount: emittedFiles.size,
    errorCount: fileErrors.size,
  }
}

// =============================================================================
// Webpack Loader-Style File Emitter
// =============================================================================

/**
 * Result of emitting a single file.
 */
export interface FileEmitResult {
  /** The compiled JavaScript code */
  content: string | undefined
  /** Source map as JSON string */
  map?: string | undefined
  /** Error message if emission failed */
  error?: string
}

/**
 * Webpack loader-style Angular file emitter.
 *
 * This mimics how AngularWebpackPlugin works:
 * 1. Creates ONE NgtscProgram with all files as rootNames
 * 2. Calls analyzeAsync() ONCE to analyze all components
 * 3. Emits ALL files at once during initialize() and caches results
 * 4. Provides per-file access through emit(filePath) method
 *
 * This is faster than creating separate programs for each file because:
 * - TypeScript only parses the project once
 * - Angular only analyzes all component metadata once
 * - Module resolution is cached across all files
 * - Emit happens once for all files (not per-file which re-runs the pipeline)
 *
 * Usage:
 * ```typescript
 * const emitter = new NgtscFileEmitter(filePaths, tsconfigPath, fileContents);
 * await emitter.initialize();
 *
 * // Get emitted files (already compiled during initialize)
 * for (const filePath of filePaths) {
 *   const result = emitter.emit(filePath);
 *   console.log(result.content); // Full output: ɵcmp, ɵfac, etc.
 * }
 * ```
 */
export class NgtscFileEmitter {
  private program: NgtscProgram | null = null
  private options: CompilerOptions | null = null
  private initialized = false
  /** Cache for emitted file contents - populated during initialize() */
  private emittedFiles: Map<string, string> = new Map()

  constructor(
    private rootNames: string[],
    private tsconfigPath: string,
    private fileContents?: Map<string, string>,
  ) {}

  /**
   * Initialize the program, run analysis, and emit ALL files.
   * This must be called once before emit().
   *
   * Emitting all files at once is much faster than per-file emit because
   * ngProgram.emit() re-runs the full compilation pipeline each time.
   */
  async initialize(): Promise<void> {
    if (this.initialized) {
      return
    }

    const startTime = performance.now()

    // Load tsconfig
    const { compilerOptions } = loadTsConfig(this.tsconfigPath)

    // Merge with Angular options - skip type checking for speed
    // Use experimental-local mode for fast compilation
    const angularOptions = createAngularOptions(true)
    this.options = {
      ...compilerOptions,
      ...angularOptions,
      compilationMode: 'experimental-local', // Fast mode for comparison
      noEmitOnError: false,
      skipLibCheck: true,
      skipDefaultLibCheck: true,
      noUnusedLocals: false,
      noUnusedParameters: false,
      strict: false,
      noImplicitAny: false,
      strictNullChecks: false,
      strictFunctionTypes: false,
      strictBindCallApply: false,
      strictPropertyInitialization: false,
      // Force ESNext target to eliminate ES transform differences (__awaiter, etc.)
      target: ts.ScriptTarget.ESNext,
      // Disable decorator metadata and experimental decorators to prevent __decorate emission
      emitDecoratorMetadata: false,
      experimentalDecorators: false,
      importHelpers: false,
    } as CompilerOptions

    // Create hybrid host (resolves @angular/* from compare tool's node_modules)
    const baseHost = ts.createCompilerHost(this.options)
    const hybridHost = createHybridResolveHost(baseHost, this.options)

    // Apply source file caching to speed up parsing
    const cachedHost = augmentHostWithSourceFileCaching(hybridHost)

    // Create host with file overrides if provided
    let host: ts.CompilerHost
    if (this.fileContents && this.fileContents.size > 0) {
      host = createMultiFileOverrideHost(cachedHost, this.fileContents)
    } else {
      host = cachedHost
    }

    const setupTime = performance.now()
    console.log(`  [NgtscFileEmitter] Setup: ${(setupTime - startTime).toFixed(0)}ms`)

    // Create ONE NgtscProgram with ALL files
    this.program = new NgtscProgram(this.rootNames, this.options, host)

    const programTime = performance.now()
    console.log(
      `  [NgtscFileEmitter] Create program (${this.rootNames.length} files): ${(programTime - setupTime).toFixed(0)}ms`,
    )

    // Call analyzeAsync() ONCE - this is the expensive operation
    await this.program.compiler.analyzeAsync()

    const analyzeTime = performance.now()
    console.log(`  [NgtscFileEmitter] Analyze: ${(analyzeTime - programTime).toFixed(0)}ms`)

    // Build a mapping from expected output paths to source paths
    // This correctly handles TypeScript's rootDir calculation and path transformations
    const outputToSource = buildOutputToSourceMap(this.rootNames, this.options)

    // Define write callback to capture emitted files
    const writeFileCallback = (fileName: string, data: string) => {
      if (fileName.endsWith('.js')) {
        // Look up the source file for this output path
        const normalizedOutput = path.normalize(fileName)
        const inputPath = outputToSource.get(normalizedOutput)
        if (inputPath) {
          this.emittedFiles.set(inputPath, data)
        }
      }
    }

    // Emit ALL files using Angular's emit pipeline with proper transformers
    // This ensures Angular's AOT transformers are properly applied
    this.program.emit({
      emitFlags: EmitFlags.Default,
      forceEmit: true,
      emitCallback: (args) => {
        return args.program.emit(
          args.targetSourceFile,
          writeFileCallback,
          args.cancellationToken,
          false, // emitOnlyDtsFiles
          args.customTransformers,
        )
      },
    })

    const emitTime = performance.now()
    console.log(`  [NgtscFileEmitter] Emit all: ${(emitTime - analyzeTime).toFixed(0)}ms`)
    console.log(
      `  [NgtscFileEmitter] Total: ${(emitTime - startTime).toFixed(0)}ms (${this.emittedFiles.size} files emitted)`,
    )

    this.initialized = true
  }

  /**
   * Get the emitted output for a single file.
   *
   * Returns the full compiled output including:
   * - ɵcmp (component definition)
   * - ɵfac (factory function)
   * - ɵdir (directive definition)
   * - setClassMetadata call
   * - All transformed class code
   *
   * Note: All files are emitted during initialize() for performance.
   * This method simply returns the cached result.
   *
   * @param filePath - Absolute path to the file to get
   * @returns The emit result with content and optional source map
   */
  emit(filePath: string): FileEmitResult {
    if (!this.program || !this.options || !this.initialized) {
      throw new Error('NgtscFileEmitter not initialized. Call initialize() first.')
    }

    // Return cached result
    const normalizedPath = path.normalize(filePath)
    const content = this.emittedFiles.get(normalizedPath)

    if (content !== undefined) {
      return { content }
    }

    // Try the original path if normalized didn't match
    const originalContent = this.emittedFiles.get(filePath)
    if (originalContent !== undefined) {
      return { content: originalContent }
    }

    return { content: undefined, error: `No emit result for ${filePath}` }
  }

  /**
   * Get the number of files in the program.
   */
  get fileCount(): number {
    return this.rootNames.length
  }

  /**
   * Check if the emitter is initialized.
   */
  get isInitialized(): boolean {
    return this.initialized
  }

  /**
   * Get the number of files that were successfully emitted.
   */
  get emittedFileCount(): number {
    return this.emittedFiles.size
  }

  /**
   * Get all emitted file paths.
   */
  get emittedFilePaths(): string[] {
    return Array.from(this.emittedFiles.keys())
  }

  /**
   * Get a copy of all emitted files as a Map.
   * Used by the ng-baseline system to snapshot Angular compilation output.
   */
  getAllEmittedFiles(): Map<string, string> {
    return new Map(this.emittedFiles)
  }
}

// =============================================================================
// Template-to-Component Compilation
// =============================================================================

/**
 * Generate a component source file from a template and metadata.
 *
 * This creates a complete Angular component TypeScript source that can be
 * compiled by NgtscProgram, matching the behavior of real Angular applications.
 *
 * @param template - The component template HTML
 * @param className - The component class name
 * @param metadata - Optional component metadata
 * @returns A complete TypeScript source file
 */
export function generateComponentSource(
  template: string,
  className: string,
  metadata?: ComponentMetadata,
): string {
  const imports = ['Component']
  const decoratorProps: string[] = []

  // Add selector
  if (metadata?.selector) {
    decoratorProps.push(`selector: '${metadata.selector}'`)
  } else {
    decoratorProps.push(`selector: 'app-${className.toLowerCase().replace('component', '')}'`)
  }

  // Add standalone
  if (metadata?.standalone !== undefined) {
    decoratorProps.push(`standalone: ${metadata.standalone}`)
  } else {
    decoratorProps.push(`standalone: true`)
  }

  // Add encapsulation
  if (metadata?.encapsulation) {
    imports.push('ViewEncapsulation')
    decoratorProps.push(`encapsulation: ViewEncapsulation.${metadata.encapsulation}`)
  }

  // Add changeDetection
  if (metadata?.changeDetection) {
    imports.push('ChangeDetectionStrategy')
    decoratorProps.push(`changeDetection: ChangeDetectionStrategy.${metadata.changeDetection}`)
  }

  // Add host bindings
  if (metadata?.host && Object.keys(metadata.host).length > 0) {
    const hostObj = JSON.stringify(metadata.host, null, 2)
      .split('\n')
      .map((line, i) => (i === 0 ? line : '    ' + line))
      .join('\n')
    decoratorProps.push(`host: ${hostObj}`)
  }

  // Add preserveWhitespaces
  if (metadata?.preserveWhitespaces !== undefined) {
    decoratorProps.push(`preserveWhitespaces: ${metadata.preserveWhitespaces}`)
  }

  // Add providers
  if (metadata?.providers) {
    decoratorProps.push(`providers: ${metadata.providers}`)
  }

  // Add viewProviders
  if (metadata?.viewProviders) {
    decoratorProps.push(`viewProviders: ${metadata.viewProviders}`)
  }

  // Add animations
  if (metadata?.animations) {
    decoratorProps.push(`animations: ${metadata.animations}`)
  }

  // Add schemas
  if (metadata?.schemas && metadata.schemas.length > 0) {
    for (const schema of metadata.schemas) {
      if (!imports.includes(schema)) {
        imports.push(schema)
      }
    }
    decoratorProps.push(`schemas: [${metadata.schemas.join(', ')}]`)
  }

  // Add imports
  if (metadata?.imports && metadata.imports.length > 0) {
    decoratorProps.push(`imports: [${metadata.imports.join(', ')}]`)
  }

  // Add hostDirectives
  if (metadata?.hostDirectives && metadata.hostDirectives.length > 0) {
    const hostDirStrings = metadata.hostDirectives.map((hd) => {
      const parts: string[] = [`directive: ${hd.directive}`]
      if (hd.inputs && hd.inputs.length > 0) {
        parts.push(`inputs: ${JSON.stringify(hd.inputs)}`)
      }
      if (hd.outputs && hd.outputs.length > 0) {
        parts.push(`outputs: ${JSON.stringify(hd.outputs)}`)
      }
      return `{ ${parts.join(', ')} }`
    })
    decoratorProps.push(`hostDirectives: [${hostDirStrings.join(', ')}]`)
  }

  // Add styles
  if (metadata?.styles && metadata.styles.length > 0) {
    const stylesArray = metadata.styles.map((s) => `\`${s}\``).join(', ')
    decoratorProps.push(`styles: [${stylesArray}]`)
  }

  // Add exportAs
  if (metadata?.exportAs) {
    decoratorProps.push(`exportAs: '${metadata.exportAs}'`)
  }

  // Add template (always last)
  // Escape backticks in template
  const escapedTemplate = template.replace(/`/g, '\\`').replace(/\${/g, '\\${')
  decoratorProps.push(`template: \`${escapedTemplate}\``)

  // Build the decorator
  const decorator = `@Component({
  ${decoratorProps.join(',\n  ')}
})`

  // Build the class
  const classBody: string[] = []

  // Add inputs/outputs as class properties if specified
  if (metadata?.inputs) {
    imports.push('input')
    for (const [_, info] of Object.entries(metadata.inputs)) {
      if (info.isSignal) {
        if (info.required) {
          classBody.push(`  ${info.classPropertyName} = input.required<any>()`)
        } else {
          classBody.push(`  ${info.classPropertyName} = input<any>()`)
        }
      }
    }
  }

  if (metadata?.outputs) {
    imports.push('output')
    for (const [_publicName, classPropertyName] of Object.entries(metadata.outputs)) {
      classBody.push(`  ${classPropertyName} = output<any>()`)
    }
  }

  // Build imports statement
  const importStatement = `import { ${imports.join(', ')} } from '@angular/core';`

  // Build the complete source
  return `${importStatement}

${decorator}
export class ${className} {
${classBody.length > 0 ? classBody.join('\n') + '\n' : ''}}
`
}

/**
 * Compile a template using NgtscProgram by generating a full component source.
 *
 * This function provides the same interface as compileWithAngular but uses
 * NgtscProgram internally for compilation, ensuring consistent behavior
 * with real Angular CLI builds.
 *
 * @param template - The template HTML to compile
 * @param className - The component class name
 * @param filePath - The file path for error reporting
 * @param metadata - Optional component metadata
 * @returns The compiled JavaScript output
 */
export async function compileTemplateWithNgtsc(
  template: string,
  className: string,
  filePath: string,
  metadata?: ComponentMetadata,
): Promise<CompilerOutput> {
  // Generate the full component source
  const source = generateComponentSource(template, className, metadata)

  // Compile with NgtscProgram
  return compileWithNgtsc(source, filePath)
}

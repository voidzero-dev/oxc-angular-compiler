/**
 * Result of validating metadata extraction between Oxc and TypeScript.
 */
export interface ExtractionValidation {
  /** File path where extraction was performed */
  filePath: string
  /** Results for each component in the file */
  componentResults: ExtractionValidationComponentResult[]
}

/**
 * Validation result for a single component's metadata extraction.
 */
export interface ExtractionValidationComponentResult {
  /** Component class name */
  className: string
  /** Whether Oxc and TypeScript extractions match */
  matches: boolean
  /** List of mismatches found (empty if matches is true) */
  mismatches: string[]
}

/**
 * Configuration for the comparison runner.
 */
export interface CompilerConfig {
  /** Root directory of the Angular project to scan */
  projectRoot: string
  /** Glob patterns to include (default: ['**\/*.component.ts']) */
  include?: string[]
  /** Glob patterns to exclude (default: ['**\/node_modules\/**', '**\/*.spec.ts']) */
  exclude?: string[]
  /** Enable parallel compilation (default: true) */
  parallel?: boolean
  /** Output file path for JSON report */
  outputPath?: string
  /** Name of the preset being used (for reporting purposes) */
  presetName?: string
  /** Enable full-file compilation mode instead of template-only (default: false) */
  fullFileMode?: boolean
  /** Path to tsconfig.json for full-file compilation with real project context */
  tsconfigPath?: string
  /** Path to load Angular baseline from (skips Angular compilation) */
  ngBaselinePath?: string
  /** Path to save Angular baseline to after compilation */
  saveNgBaselinePath?: string
  /** Run Angular compilation only and save baseline (no Oxc, no comparison) */
  generateNgBaselineOnly?: boolean
}

/**
 * Host metadata extracted from the `host` property of a @Component decorator.
 */
export interface HostMetadata {
  /** Host property bindings: [[key, value], ...] */
  properties: string[][]
  /** Host attribute bindings: [[key, value], ...] */
  attributes: string[][]
  /** Host event listeners: [[key, value], ...] */
  listeners: string[][]
  /** Static class attribute binding (from `class` key in host). */
  classAttr?: string
  /** Static style attribute binding (from `style` key in host). */
  styleAttr?: string
}

/**
 * Host directive metadata extracted from the `hostDirectives` property.
 */
export interface HostDirectiveInfo {
  /** The directive class name. */
  directive: string
  /** Input mappings: [[publicName, internalName], ...] */
  inputs: string[][]
  /** Output mappings: [[publicName, internalName], ...] */
  outputs: string[][]
  /** Whether this is a forward reference. */
  isForwardReference: boolean
}

/**
 * Information about a discovered Angular component.
 */
export interface ComponentInfo {
  /** Absolute path to the component TypeScript file */
  filePath: string
  /** Component class name (e.g., 'AppComponent') */
  className: string
  /** Template HTML content (resolved from inline or external file) */
  templateContent: string
  /** Original templateUrl from decorator (e.g., "./foo.component.html") */
  templateUrl?: string
  /** Absolute path to external template file (if using templateUrl) */
  templatePath?: string
  /** Original styleUrls from decorator (e.g., ["./foo.component.scss"]) */
  originalStyleUrls?: string[]
  /** Absolute paths to external style files (resolved from styleUrls) */
  styleUrls?: string[]
  /** Inline styles array */
  styles?: string[]
  /** The CSS selector that identifies this component in a template */
  selector?: string
  /** Whether this is a standalone component */
  standalone?: boolean
  /** View encapsulation mode: "Emulated" | "None" | "ShadowDom" */
  encapsulation?: 'Emulated' | 'None' | 'ShadowDom'
  /** Change detection strategy: "Default" | "OnPush" */
  changeDetection?: 'Default' | 'OnPush'
  /** Host bindings and listeners */
  host?: HostMetadata
  /** Component imports (for standalone components) */
  imports?: string[]
  /** Exported names for template references */
  exportAs?: string
  /** Whether to preserve whitespace in templates */
  preserveWhitespaces?: boolean
  /** Providers expression as emitted JavaScript */
  providers?: string
  /** View providers expression as emitted JavaScript */
  viewProviders?: string
  /** Animations expression as emitted JavaScript */
  animations?: string
  /** Schema identifiers (e.g., "CUSTOM_ELEMENTS_SCHEMA") */
  schemas?: string[]
  /** Host directives configuration */
  hostDirectives?: HostDirectiveInfo[]
  /** Whether to use external i18n message IDs (default: true) */
  i18nUseExternalIds?: boolean
  /** Full source code of the file (for full-file transformation) */
  sourceCode?: string
  /**
   * Component inputs - map of public name to binding info.
   * For template-only comparison this is typically not available.
   */
  inputs?: Record<string, InputBindingInfo>
  /**
   * Component outputs - map of public name to emitter name.
   * For template-only comparison this is typically not available.
   */
  outputs?: Record<string, string>
  /**
   * Content queries (@ContentChild, @ContentChildren).
   * For template-only comparison this is typically not available.
   */
  queries?: QueryInfo[]
  /**
   * View queries (@ViewChild, @ViewChildren).
   * For template-only comparison this is typically not available.
   */
  viewQueries?: QueryInfo[]
}

/**
 * Input binding information extracted from @Input() decorator.
 */
export interface InputBindingInfo {
  /** Binding property name (internal field name) */
  bindingPropertyName: string
  /** Class property name */
  classPropertyName: string
  /** Whether this is a required input */
  required: boolean
  /** Whether this is a signal-based input */
  isSignal: boolean
  /** Transform function name (if any) */
  transform?: string
}

/**
 * Query information extracted from @ViewChild/@ContentChild decorators.
 */
export interface QueryInfo {
  /** Property name where query result is stored */
  propertyName: string
  /** Query predicate (selector or type) */
  predicate: string | string[]
  /** Whether to query descendants (for ContentChild/Children) */
  descendants?: boolean
  /** Whether this is a static query */
  static?: boolean
  /** Whether this reads a specific token */
  read?: string
}

/**
 * Output from a single compiler.
 */
export interface CompilerOutput {
  /** Generated JavaScript code */
  code: string
  /** Error message if compilation failed */
  error?: string
  /** Time taken to compile in milliseconds */
  compilationTimeMs: number
}

/**
 * A single difference in the AST comparison.
 */
export interface AstDiff {
  /** Path to the differing node (e.g., 'body[0].declarations[0].init') */
  path: string
  /** Type of difference */
  type: 'missing' | 'extra' | 'different'
  /** Expected value (from TypeScript compiler) */
  expected?: string
  /** Actual value (from Oxc compiler) */
  actual?: string
}

/**
 * Detailed comparison of a single function that exists in both outputs.
 */
export interface FunctionDiff {
  /** Name of the function */
  name: string
  /** Actual Oxc function code */
  oxcCode: string
  /** Actual TypeScript function code */
  tsCode: string
  /** AST differences within the function (deprecated, kept for compatibility) */
  diffs: AstDiff[]
}

/**
 * Function-level comparison result showing which functions match, differ, or are missing/extra.
 */
export interface FunctionLevelComparison {
  /** Function names present in TypeScript output but missing in Oxc output */
  missingFunctions: string[]
  /** Function names present in Oxc output but not in TypeScript output */
  extraFunctions: string[]
  /** Functions that exist in both but have AST differences */
  functionDiffs: FunctionDiff[]
  /** Function names that match exactly */
  matchingFunctions: string[]
}

/**
 * Result of comparing a single component.
 */
export interface CompilationResult {
  /** Component that was compiled */
  component: ComponentInfo
  /** Overall status of the comparison */
  status: 'match' | 'mismatch' | 'oxc-error' | 'ts-error' | 'both-error' | 'angular-jit-only'
  /** Oxc compiler output (included for non-matching results) */
  oxcOutput?: CompilerOutput
  /** TypeScript compiler output (included for non-matching results) */
  tsOutput?: CompilerOutput
  /** Function-level comparison results (for mismatches) */
  functionComparison?: FunctionLevelComparison
  /** Legacy AST differences (deprecated, use functionComparison) */
  diff?: AstDiff[]
}

/**
 * Summary statistics for the comparison run.
 */
export interface ReportSummary {
  /** Total number of components processed */
  total: number
  /** Number of components with matching output */
  matched: number
  /** Number of components with different output */
  mismatched: number
  /** Number of components where only Oxc failed */
  oxcErrors: number
  /** Number of components where only TS failed */
  tsErrors: number
  /** Number of components where both failed */
  bothErrors: number
  /** Number of components where Angular produced JIT output instead of AOT */
  angularJitOnly: number
  /** Pass rate as percentage (matched / total * 100) */
  passRate: number
}

/**
 * Metadata about the comparison run.
 */
export interface ReportMetadata {
  /** ISO timestamp when report was generated */
  generatedAt: string
  /** Root directory that was scanned */
  projectRoot: string
  /** Total duration of the comparison in milliseconds */
  durationMs: number
  /** Oxc compiler version */
  oxcVersion?: string
  /** Angular compiler version */
  angularVersion?: string
}

/**
 * Complete comparison report.
 */
export interface ComparisonReport {
  /** Summary statistics (component-level) */
  summary: ReportSummary
  /** Run metadata */
  metadata: ReportMetadata
  /** Individual component results (template-only or component-level comparison) */
  results: CompilationResult[]
  /** File-level comparison mode flag */
  fileComparisonMode?: boolean
  /** File-level summary statistics (only in file comparison mode) */
  fileSummary?: FileReportSummary
  /** File-level comparison results (only in file comparison mode) */
  fileResults?: FileComparisonResult[]
}

/**
 * Difference in setClassMetadata calls between Oxc and TS outputs.
 */
export interface ClassMetadataDiff {
  /** Type of difference */
  type: 'missing' | 'extra' | 'different'
  /** Class name */
  className: string
  /** Which field is different (e.g., "setClassMetadata", "decorators", "ctorParams", "propDecorators") */
  field: string
  /** Expected value (from TS) */
  expected?: string
  /** Actual value (from Oxc) */
  actual?: string
}

/**
 * Result from compiling an entire project with a single compiler pass.
 * Used for project-wide NgtscProgram compilation and batch Oxc compilation.
 */
export interface ProjectCompilationResult {
  /** Map of source file path -> emitted JavaScript code */
  emittedFiles: Map<string, string>
  /** Map of source file path -> error messages */
  errors: Map<string, string[]>
  /** Total compilation time in milliseconds */
  durationMs: number
  /** Number of files successfully compiled */
  successCount: number
  /** Number of files that failed to compile */
  errorCount: number
}

/**
 * Result of comparing a single file's full output (file-to-file comparison).
 * Used in --full-file mode for strict alignment with NgtscProgram output.
 */
export interface FileComparisonResult {
  /** File path that was compared */
  filePath: string
  /** Component class names in this file */
  classNames: string[]
  /** Overall status of the file comparison */
  status: 'match' | 'mismatch' | 'oxc-error' | 'ng-error' | 'both-error' | 'angular-jit-only'
  /** Full Oxc output for the file */
  oxcOutput?: string
  /** Full Ngtsc output for the file */
  ngOutput?: string
  /** Error from Oxc compilation (if any) */
  oxcError?: string
  /** Error from Angular compilation (if any) */
  ngError?: string
  /** Detailed comparison result (for mismatches) */
  comparisonDetails?: FileComparisonDetails
  /** Compilation time for Oxc in milliseconds */
  oxcCompilationTimeMs?: number
  /** Compilation time for Angular in milliseconds */
  ngCompilationTimeMs?: number
}

/**
 * Detailed comparison results for file-to-file comparison.
 * Imported from compare.ts to avoid duplication.
 */
export interface FileComparisonDetails {
  /** Import differences */
  importDiffs?: Array<{
    type: 'missing' | 'extra' | 'different'
    moduleSource: string
    expected?: string[]
    actual?: string[]
  }>
  /** Export differences */
  exportDiffs?: Array<{
    type: 'missing' | 'extra' | 'different'
    exportName: string
    expected?: string
    actual?: string
  }>
  /** Class definition differences */
  classDiffs?: Array<{
    className: string
    type: 'missing' | 'extra' | 'different'
    description: string
  }>
  /** Static field assignment differences */
  staticFieldDiffs?: Array<{
    className: string
    fieldName: string
    type: 'missing' | 'extra' | 'different'
    expected?: string
    actual?: string
  }>
  /** Class metadata differences */
  classMetadataDiffs?: ClassMetadataDiff[]
  /** Function comparison results */
  functionComparison?: FunctionLevelComparison
  /** Parse errors if any */
  parseErrors?: string[]
}

/**
 * Summary statistics for file-level comparison (used in --full-file mode).
 */
export interface FileReportSummary {
  /** Total number of files compared */
  totalFiles: number
  /** Number of files with matching output */
  matchedFiles: number
  /** Number of files with different output */
  mismatchedFiles: number
  /** Number of files where only Oxc failed */
  oxcErrors: number
  /** Number of files where only Angular failed */
  ngErrors: number
  /** Number of files where both failed */
  bothErrors: number
  /** Number of files where Angular produced JIT output instead of AOT */
  angularJitOnly: number
  /** File-level pass rate as percentage */
  passRate: number
  /** Total number of components across all files */
  totalComponents: number
}

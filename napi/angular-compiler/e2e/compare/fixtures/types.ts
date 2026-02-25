/**
 * Fixture system types for Angular compiler comparison testing.
 *
 * Fixtures are standalone Angular component templates that test specific
 * compiler features not covered by real-world project testing.
 */

import type { TransformOptions } from '@oxc-angular/vite/api'

import type { ImportDiff, ExportDiff, ClassDiff, StaticFieldDiff } from '../src/compare.js'
import type { FunctionLevelComparison, AstDiff, ClassMetadataDiff } from '../src/types.js'

/**
 * Fixture type discriminator for different compilation modes.
 */
export type FixtureType =
  | 'component'
  | 'full-transform'
  | 'directive'
  | 'pipe'
  | 'injectable'
  | 'ng-module'
  | 'injector'
  | 'factory'
  | 'class-metadata'

/**
 * Host directive configuration for the hostDirectives property.
 */
export interface HostDirectiveConfig {
  /** The directive class name */
  directive: string

  /** Input mappings: [[publicName, privateName], ...] */
  inputs?: string[][]

  /** Output mappings: [[publicName, privateName], ...] */
  outputs?: string[][]

  /** Whether this directive reference uses forwardRef() */
  isForwardReference?: boolean
}

/**
 * Component metadata fields that can be used for full compilation testing.
 * These mirror Angular's @Component decorator options.
 */
export interface ComponentMetadata {
  /** The CSS selector that identifies this component in a template */
  selector?: string

  /** Whether this component is standalone (default: true in modern Angular) */
  standalone?: boolean

  /** View encapsulation mode */
  encapsulation?: 'Emulated' | 'None' | 'ShadowDom'

  /** Change detection strategy */
  changeDetection?: 'Default' | 'OnPush'

  /** Host bindings and listeners */
  host?: Record<string, string>

  /** Whether to preserve whitespace in templates */
  preserveWhitespaces?: boolean

  /** Serialized provider expression */
  providers?: string

  /** Serialized view provider expression */
  viewProviders?: string

  /** Serialized animations expression */
  animations?: string

  /** Schema identifiers (e.g., "CUSTOM_ELEMENTS_SCHEMA") */
  schemas?: string[]

  /** Component imports for standalone components */
  imports?: string[]

  /** Host directive configuration */
  hostDirectives?: HostDirectiveConfig[]

  /** Inline styles array (CSS strings) */
  styles?: string[]

  /** Exported names for template references */
  exportAs?: string

  /** Whether to use external i18n message IDs (default: true) */
  i18nUseExternalIds?: boolean

  /**
   * Component inputs - map of public name to binding info.
   * Typically extracted from @Input() decorators on the class.
   */
  inputs?: Record<string, InputBindingInfo>

  /**
   * Component outputs - map of public name to emitter name.
   * Typically extracted from @Output() decorators on the class.
   */
  outputs?: Record<string, string>

  /**
   * Content queries (@ContentChild, @ContentChildren).
   * Typically extracted from query decorators on the class.
   */
  queries?: QueryInfo[]

  /**
   * View queries (@ViewChild, @ViewChildren).
   * Typically extracted from query decorators on the class.
   */
  viewQueries?: QueryInfo[]
}

/**
 * Input binding information.
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
 * Query information.
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
 * Definition of a single test fixture.
 */
export interface Fixture {
  /** Fixture type - defaults to "component" for template compilation */
  type?: FixtureType

  /** Unique identifier for the fixture (e.g., 'basic-defer') */
  name: string

  /** Category for grouping (e.g., 'defer', 'animations', 'i18n') */
  category: string

  /** Human-readable description of what this fixture tests */
  description: string

  /** HTML template content to compile (for component fixtures) */
  template?: string

  /** Full TypeScript source code (for decorator fixtures like pipe, injectable) */
  sourceCode?: string

  /** Component class name (e.g., 'BasicDeferComponent') */
  className: string

  /**
   * Decorator type for class-metadata fixtures.
   * Specifies which Angular decorator to process: "Component", "Directive", "Pipe", "Injectable", "NgModule"
   */
  decoratorType?: 'Component' | 'Directive' | 'Pipe' | 'Injectable' | 'NgModule'

  /** Expected R3 function names or patterns in the compiled output */
  expectedFeatures?: string[]

  /** Skip this fixture during comparison */
  skip?: boolean

  /** Reason for skipping (displayed in reports) */
  skipReason?: string

  /** Custom compiler options to override defaults */
  compilerOptions?: Partial<TransformOptions>

  /** Optional file path override (generated from category/name if not provided) */
  filePath?: string

  // Component metadata for full compilation testing

  /** The CSS selector that identifies this component in a template */
  selector?: string

  /** Whether this component is standalone (default: true in modern Angular) */
  standalone?: boolean

  /** View encapsulation mode */
  encapsulation?: 'Emulated' | 'None' | 'ShadowDom'

  /** Change detection strategy */
  changeDetection?: 'Default' | 'OnPush'

  /** Host bindings and listeners */
  host?: Record<string, string>

  /** Whether to preserve whitespace in templates */
  preserveWhitespaces?: boolean

  /** Serialized provider expression */
  providers?: string

  /** Serialized view provider expression */
  viewProviders?: string

  /** Serialized animations expression */
  animations?: string

  /** Schema identifiers (e.g., "CUSTOM_ELEMENTS_SCHEMA") */
  schemas?: string[]

  /** Component imports for standalone components */
  imports?: string[]

  /** Host directive configuration */
  hostDirectives?: HostDirectiveConfig[]

  /** Inline styles array (CSS strings) */
  styles?: string[]

  /** Exported names for template references */
  exportAs?: string

  /** Enable HMR (Hot Module Replacement) compilation mode */
  hmrEnabled?: boolean

  /** Whether to use external i18n message IDs (default: true) */
  i18nUseExternalIds?: boolean
}

/**
 * Output from compiling a fixture with either compiler.
 */
export interface FixtureCompilerOutput {
  /** Generated JavaScript code */
  code: string

  /** Error message if compilation failed */
  error?: string

  /** Compilation time in milliseconds */
  compilationTimeMs: number
}

/**
 * Result of comparing a single fixture.
 */
export interface FixtureResult {
  /** The fixture that was tested */
  fixture: Fixture

  /** Comparison status */
  status: 'match' | 'mismatch' | 'oxc-error' | 'ts-error' | 'both-error' | 'skipped'

  /** Skip reason (only when status is "skipped") */
  skipReason?: string

  /** Oxc compiler output */
  oxcOutput?: FixtureCompilerOutput

  /** TypeScript Angular compiler output */
  tsOutput?: FixtureCompilerOutput

  /** Function-level comparison details (for mismatches) */
  functionComparison?: FunctionLevelComparison

  /** AST differences found */
  diff?: AstDiff[]

  /** Import differences (for full-file comparison) */
  importDiffs?: ImportDiff[]

  /** Export differences (for full-file comparison) */
  exportDiffs?: ExportDiff[]

  /** Class definition differences (for full-file comparison) */
  classDiffs?: ClassDiff[]

  /** Static field assignment differences (for full-file comparison) */
  staticFieldDiffs?: StaticFieldDiff[]

  /** Class metadata (setClassMetadata) differences (for full-file comparison) */
  classMetadataDiffs?: ClassMetadataDiff[]

  /** Parse errors during comparison (for full-file comparison) */
  parseErrors?: string[]

  /** Expected features found in the Oxc output */
  oxcFeatures?: string[]

  /** Expected features found in the Angular/TypeScript output */
  tsFeatures?: string[]

  /** Features present in one compiler output but not the other */
  asymmetricFeatures?: string[]

  /** Expected features missing from BOTH outputs (test configuration issue) */
  missingFromBoth?: string[]
}

/**
 * Category-level statistics.
 */
export interface CategoryStats {
  /** Total fixtures in this category */
  total: number

  /** Number of passing fixtures */
  passed: number

  /** Number of failing fixtures */
  failed: number

  /** Number of skipped fixtures */
  skipped: number

  /** Pass rate as percentage */
  passRate: number
}

/**
 * Complete fixture test report.
 */
export interface FixtureReport {
  /** Summary statistics */
  summary: {
    /** Total number of fixtures */
    total: number

    /** Fixtures where Oxc matches TypeScript */
    matched: number

    /** Fixtures where outputs differ */
    mismatched: number

    /** Fixtures where Oxc failed to compile */
    oxcErrors: number

    /** Fixtures where TypeScript failed to compile */
    tsErrors: number

    /** Fixtures where both compilers failed */
    bothErrors: number

    /** Fixtures that were skipped */
    skipped: number

    /** Overall pass rate matched / (total - skipped) */
    passRate: number

    /** Statistics broken down by category */
    byCategory: Record<string, CategoryStats>
  }

  /** Metadata about the test run */
  metadata: {
    /** When the test was run */
    generatedAt: string

    /** Total execution time in milliseconds */
    durationMs: number

    /** Categories that were tested (empty = all) */
    categories: string[]

    /** Oxc compiler version */
    oxcVersion?: string

    /** Angular compiler version */
    angularVersion?: string
  }

  /** Individual fixture results */
  fixtures: FixtureResult[]
}

/**
 * Options for running fixture tests.
 */
export interface FixtureRunnerOptions {
  /** Only run fixtures in these categories */
  categories?: string[]

  /** Enable verbose output */
  verbose?: boolean

  /** Run fixtures in parallel */
  parallel?: boolean

  /** Output path for the report */
  outputPath?: string
}

// =============================================================================
// Decorator Fixture Types
// =============================================================================

/**
 * Base interface for decorator-based fixtures.
 */
export interface BaseDecoratorFixture {
  /** Unique identifier for the fixture */
  name: string

  /** Category for grouping */
  category: string

  /** Human-readable description of what this fixture tests */
  description: string

  /** Full TypeScript source code containing the decorator */
  sourceCode: string

  /** The class name to compile */
  className: string

  /** Expected R3 function names or patterns in the compiled output */
  expectedFeatures?: string[]

  /** Skip this fixture during comparison */
  skip?: boolean

  /** Reason for skipping */
  skipReason?: string

  /** Optional file path override */
  filePath?: string
}

/**
 * Directive fixture - tests @Directive decorator compilation.
 */
export interface DirectiveFixture extends BaseDecoratorFixture {
  type: 'directive'
}

/**
 * Pipe fixture - tests @Pipe decorator compilation.
 */
export interface PipeFixture extends BaseDecoratorFixture {
  type: 'pipe'
  /** Whether standalone defaults to true (Angular v19+) */
  implicitStandalone?: boolean
}

/**
 * Injectable fixture - tests @Injectable decorator compilation.
 */
export interface InjectableFixture extends BaseDecoratorFixture {
  type: 'injectable'
}

/**
 * NgModule fixture - tests @NgModule decorator compilation.
 */
export interface NgModuleFixture extends BaseDecoratorFixture {
  type: 'ng-module'
}

/**
 * Injector fixture - tests injector compilation from metadata.
 */
export interface InjectorFixture {
  type: 'injector'
  name: string
  category: string
  description: string
  /** The injector name (typically NgModule class name) */
  injectorName: string
  /** Providers expression (variable name) */
  providers?: string
  /** Import class names */
  imports?: string[]
  /** Expected features in output */
  expectedFeatures?: string[]
  skip?: boolean
  skipReason?: string
}

/**
 * Factory fixture - tests factory function compilation.
 */
export interface FactoryFixture {
  type: 'factory'
  name: string
  category: string
  description: string
  /** The class name for the factory */
  factoryName: string
  /** Target type: Component, Directive, Injectable, Pipe, NgModule */
  target?: string
  /** Deps kind: Valid, Invalid, None */
  depsKind?: string
  /** Dependencies for the factory */
  deps?: Array<{
    token?: string
    attributeNameType?: string
    host?: boolean
    optional?: boolean
    self?: boolean
    skipSelf?: boolean
  }>
  /** Expected features in output */
  expectedFeatures?: string[]
  skip?: boolean
  skipReason?: string
}

/**
 * Full transform fixture - tests transformAngularFile with complete TypeScript source.
 *
 * This fixture type uses the full-file transformation API instead of template-only
 * compilation, allowing testing of class-level decorators (@HostBinding, @HostListener),
 * proper DI resolution, and full component semantics.
 */
export interface FullTransformFixture extends BaseDecoratorFixture {
  type: 'full-transform'
}

/**
 * Full file fixture - tests complete TypeScript source file transformation.
 *
 * This is a type alias for FullTransformFixture, used for fixtures that contain
 * complete Angular component files including:
 * - Import statements
 * - Component decorator with all metadata
 * - Class definition with constructor, properties, and methods
 *
 * The `sourceCode` field contains the full TypeScript source.
 */
export type FullFileFixture = FullTransformFixture

/**
 * Union type for all decorator fixtures.
 */
export type DecoratorFixture =
  | DirectiveFixture
  | PipeFixture
  | InjectableFixture
  | NgModuleFixture
  | InjectorFixture
  | FactoryFixture
  | FullTransformFixture

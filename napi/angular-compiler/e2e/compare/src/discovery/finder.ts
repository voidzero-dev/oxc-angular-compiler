import { readFile } from 'fs/promises'
import { cpus } from 'node:os'
import { dirname, resolve } from 'path'

import type {
  ComponentHostMetadata,
  ExtractedHostDirective,
  ExtractedComponentMetadata,
  ExtractedInputMetadata,
  ExtractedOutputMetadata,
  ExtractedQueryMetadata,
} from '@oxc-angular/vite/api'
import fg from 'fast-glob'
import pLimit from 'p-limit'

import type {
  CompilerConfig,
  ComponentInfo,
  HostMetadata,
  HostDirectiveInfo,
  InputBindingInfo,
  QueryInfo,
} from '../types.js'
import { extractComponentMetadataWithTypescript } from './typescript-extractor.js'

// Re-export types for convenience
export type { ComponentInfo, HostMetadata, HostDirectiveInfo } from '../types.js'

/**
 * Find all Angular components in a project.
 */
export async function findComponents(config: CompilerConfig): Promise<ComponentInfo[]> {
  const patterns = config.include || ['**/*.component.ts']
  const ignore = config.exclude || ['**/node_modules/**', '**/*.spec.ts', '**/*.test.ts']

  const files = await fg(patterns, {
    cwd: config.projectRoot,
    ignore,
    absolute: true,
  })

  console.log(`Found ${files.length} potential component files`)

  // Use higher concurrency for I/O-bound file reading
  // At least 8 workers, or 2x CPU cores for better throughput
  const concurrency = Math.max(cpus().length * 2, 8)
  const limit = pLimit(concurrency)

  // Process files in parallel with concurrency limit
  // In full-file mode, we don't need to store sourceCode in ComponentInfo
  // because it will be re-read into fileContents Map anyway
  const skipSourceCode = config.fullFileMode === true
  const fileResults = await Promise.all(
    files.map((filePath) =>
      limit(async () => {
        try {
          return await extractComponentsFromFile(filePath, skipSourceCode)
        } catch (e) {
          console.warn(`Warning: Could not process ${filePath}: ${String(e)}`)
          return []
        }
      }),
    ),
  )

  // Flatten results
  const components: ComponentInfo[] = fileResults.flat()

  console.log(`Extracted ${components.length} components with templates`)

  return components
}

/**
 * Extract component info from a TypeScript file using NAPI.
 * Also performs parallel extraction with TypeScript parser for validation.
 *
 * @param filePath - Path to the TypeScript file
 * @param skipSourceCode - If true, don't store sourceCode in ComponentInfo (saves memory in full-file mode)
 */
async function extractComponentsFromFile(
  filePath: string,
  skipSourceCode = false,
): Promise<ComponentInfo[]> {
  const source = await readFile(filePath, 'utf-8')

  // Quick check: skip files that don't import from @angular/core
  // This handles aliased decorators like: import { Component as Comp } from '@angular/core'
  if (!source.includes('@angular/core')) {
    return []
  }

  // NOTE: extractComponentMetadataSync NAPI binding is not yet implemented.
  // Using TypeScript-based extraction as a fallback.
  // TODO: Switch back to NAPI extraction when extractComponentMetadataSync is available.
  let metadataList: ExtractedComponentMetadata[]
  try {
    const tsMetadataList = extractComponentMetadataWithTypescript(source, filePath)
    // Convert TsExtractedComponentMetadata to ExtractedComponentMetadata format
    metadataList = tsMetadataList.map((ts) => ({
      className: ts.className,
      spanStart: 0,
      spanEnd: 0,
      selector: ts.selector,
      template: ts.template,
      templateUrl: ts.templateUrl,
      styles: ts.styles || [],
      styleUrls: ts.styleUrls || [],
      standalone: ts.standalone ?? true,
      encapsulation: ts.encapsulation || 'Emulated',
      changeDetection: ts.changeDetection || 'Default',
      // ts.host is TsExtractedHostMetadata with properties/attributes/listeners as string[][]
      host: ts.host
        ? {
            properties: ts.host.properties || [],
            attributes: ts.host.attributes || [],
            listeners: ts.host.listeners || [],
            classAttr: ts.host.classAttr,
            styleAttr: ts.host.styleAttr,
          }
        : undefined,
      imports: ts.imports || [],
      exportAs: ts.exportAs,
      preserveWhitespaces: ts.preserveWhitespaces ?? false,
      providers: ts.providers,
      viewProviders: ts.viewProviders,
      animations: ts.animations,
      schemas: ts.schemas || [],
      hostDirectives: ts.hostDirectives || [],
      inputs: undefined,
      outputs: undefined,
      queries: undefined,
      viewQueries: undefined,
    }))
  } catch (e) {
    console.warn(`Warning: Could not extract metadata from ${filePath}: ${String(e)}`)
    return []
  }

  // Extraction validation is disabled since we're using TypeScript extraction directly
  // (no NAPI extraction to compare against)

  const components: ComponentInfo[] = []

  for (const metadata of metadataList) {
    let templateContent = metadata.template || ''
    let templatePath: string | undefined

    // If external templateUrl, read the file
    if (metadata.templateUrl) {
      templatePath = resolve(dirname(filePath), metadata.templateUrl)
      try {
        templateContent = await readFile(templatePath, 'utf-8')
      } catch {
        console.warn(
          `Warning: Could not read template ${metadata.templateUrl} for ${metadata.className}`,
        )
        continue
      }
    }

    // Skip components without templates
    if (!templateContent) {
      continue
    }

    // Convert style URLs to absolute paths and load contents in parallel
    // Keep original relative URLs for Oxc resource resolution
    let originalStyleUrls: string[] | undefined
    let styleUrls: string[] | undefined
    let loadedStyles: string[] = []

    if (metadata.styleUrls.length > 0) {
      originalStyleUrls = [...metadata.styleUrls]
      styleUrls = metadata.styleUrls.map((url) => resolve(dirname(filePath), url))

      // Read all style files in parallel
      // Falls back to .scss if .css file doesn't exist (material-angular has .scss sources)
      const styleResults = await Promise.all(
        styleUrls.map(async (stylePath, index) => {
          try {
            return await readFile(stylePath, 'utf-8')
          } catch {
            // If .css file doesn't exist, try .scss fallback
            if (stylePath.endsWith('.css')) {
              const scssPath = stylePath.replace(/\.css$/, '.scss')
              try {
                return await readFile(scssPath, 'utf-8')
              } catch {
                // Both .css and .scss failed
              }
            }
            console.warn(
              `Warning: Could not read style ${metadata.styleUrls[index]} for ${metadata.className}`,
            )
            return null
          }
        }),
      )

      // Filter out failed reads and preserve order
      loadedStyles = styleResults.filter((content): content is string => content !== null)
    }

    // Combine loaded external styles with inline styles
    const allStyles = [...loadedStyles, ...metadata.styles]

    // Convert host metadata
    const host = convertHostMetadata(metadata.host)

    // Convert host directives
    const hostDirectives = convertHostDirectives(metadata.hostDirectives)

    // Convert inputs, outputs, and queries from NAPI extraction
    const inputs = convertInputs(metadata.inputs)
    const outputs = convertOutputs(metadata.outputs)
    const queries = convertQueries(metadata.queries)
    const viewQueries = convertQueries(metadata.viewQueries)

    components.push({
      filePath,
      className: metadata.className,
      templateContent,
      templateUrl: metadata.templateUrl ?? undefined,
      templatePath,
      originalStyleUrls,
      styleUrls,
      styles: allStyles.length > 0 ? allStyles : undefined,
      selector: metadata.selector ?? undefined,
      standalone: metadata.standalone,
      encapsulation: metadata.encapsulation as 'Emulated' | 'None' | 'ShadowDom',
      changeDetection: metadata.changeDetection as 'Default' | 'OnPush',
      host,
      imports: metadata.imports.length > 0 ? metadata.imports : undefined,
      exportAs: metadata.exportAs ?? undefined,
      preserveWhitespaces: metadata.preserveWhitespaces,
      providers: metadata.providers ?? undefined,
      viewProviders: metadata.viewProviders ?? undefined,
      animations: metadata.animations ?? undefined,
      schemas: metadata.schemas.length > 0 ? metadata.schemas : undefined,
      hostDirectives,
      // In full-file mode, don't store sourceCode to save ~500MB-2GB of memory
      // The source will be re-read into fileContents Map in compareFilesProjectWide
      sourceCode: skipSourceCode ? undefined : source,
      inputs,
      outputs,
      queries,
      viewQueries,
    })
  }

  return components
}

/**
 * Convert NAPI ComponentHostMetadata to local HostMetadata.
 */
function convertHostMetadata(
  napiHost: ComponentHostMetadata | null | undefined,
): HostMetadata | undefined {
  if (!napiHost) {
    return undefined
  }

  return {
    properties: napiHost.properties,
    attributes: napiHost.attributes,
    listeners: napiHost.listeners,
    classAttr: napiHost.classAttr ?? undefined,
    styleAttr: napiHost.styleAttr ?? undefined,
  }
}

/**
 * Convert NAPI ExtractedHostDirective[] to local HostDirectiveInfo[].
 */
function convertHostDirectives(
  napiDirectives: ExtractedHostDirective[],
): HostDirectiveInfo[] | undefined {
  if (napiDirectives.length === 0) {
    return undefined
  }

  return napiDirectives.map((hd) => ({
    directive: hd.directive,
    inputs: hd.inputs,
    outputs: hd.outputs,
    isForwardReference: hd.isForwardReference,
  }))
}

/**
 * Convert NAPI ExtractedInputMetadata[] to local Record<string, InputBindingInfo>.
 * The record is keyed by the public binding property name.
 */
function convertInputs(
  napiInputs: ExtractedInputMetadata[] | null | undefined,
): Record<string, InputBindingInfo> | undefined {
  if (!napiInputs || napiInputs.length === 0) {
    return undefined
  }

  const result: Record<string, InputBindingInfo> = {}
  for (const input of napiInputs) {
    result[input.bindingPropertyName] = {
      bindingPropertyName: input.bindingPropertyName,
      classPropertyName: input.classPropertyName,
      required: input.required,
      isSignal: input.isSignal,
      transform: input.transform ?? undefined,
    }
  }
  return result
}

/**
 * Convert NAPI ExtractedOutputMetadata[] to local Record<string, string>.
 * The record maps public binding property name to class property name.
 */
function convertOutputs(
  napiOutputs: ExtractedOutputMetadata[] | null | undefined,
): Record<string, string> | undefined {
  if (!napiOutputs || napiOutputs.length === 0) {
    return undefined
  }

  const result: Record<string, string> = {}
  for (const output of napiOutputs) {
    result[output.bindingPropertyName] = output.classPropertyName
  }
  return result
}

/**
 * Convert NAPI ExtractedQueryMetadata[] to local QueryInfo[].
 */
function convertQueries(
  napiQueries: ExtractedQueryMetadata[] | null | undefined,
): QueryInfo[] | undefined {
  if (!napiQueries || napiQueries.length === 0) {
    return undefined
  }

  return napiQueries.map((q) => ({
    propertyName: q.propertyName,
    predicate: q.predicate,
    descendants: q.descendants ?? undefined,
    static: q.static ?? undefined,
    read: q.read ?? undefined,
  }))
}

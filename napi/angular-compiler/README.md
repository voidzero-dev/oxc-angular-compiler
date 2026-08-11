# OXC Angular Compiler

A high-performance, Rust-based Angular compiler for Node.js. Provides both a standalone transformer and a Vite plugin for Angular applications.

## Features

- **Fast compilation** - Rust-based implementation via NAPI-RS
- **Vite integration** - Full-featured Vite plugin with HMR support
- **Template compilation** - Angular template to JavaScript transformation
- **Style encapsulation** - ViewEncapsulation.Emulated support
- **Cross-file elision** - Type-only import detection and removal
- **Drop-in replacement** - Compatible API with @angular/compiler-cli

## Installation

```bash
npm install @oxc-angular/vite
# or
pnpm add @oxc-angular/vite
```

## Quick Start

### Vite Plugin

```typescript
// vite.config.ts
import { defineConfig } from 'vite'
import { angular } from '@oxc-angular/vite/vite-plugin'

export default defineConfig({
  plugins: [
    angular({
      tsconfig: './tsconfig.json',
      sourceMap: true,
    }),
  ],
})
```

### Standalone Transformer

```typescript
import {
  transformAngularFile,
  compileTemplateSync,
  extractComponentUrlsSync,
} from '@oxc-angular/vite'

// Transform an entire Angular file
const result = await transformAngularFile(sourceCode, 'app.component.ts', {
  sourcemap: true,
})

// Compile a template only
const template = compileTemplateSync('<div>{{ title }}</div>', 'AppComponent', 'app.component.ts')
```

## API Reference

### Package Exports

| Export                  | Description   |
| ----------------------- | ------------- |
| `@oxc-angular/vite`     | Vite plugin   |
| `@oxc-angular/vite/api` | Low level API |

### Core Functions

#### Template Compilation

```typescript
// Asynchronous
compileTemplate(
  template: string,
  componentName: string,
  filePath: string,
  options?: TemplateCompileOptions
): Promise<TemplateCompileResult>
```

#### Full File Transformation

```typescript
transformAngularFile(
  source: string,
  filename: string,
  options?: TransformOptions,
  resolvedResources?: ResolvedResources
): Promise<TransformResult>
```

#### Component Metadata Extraction

```typescript
// Extract templateUrl and styleUrls
extractComponentUrlsSync(
  source: string,
  filename: string
): ComponentUrls[]
```

#### Style Processing

```typescript
// Apply ViewEncapsulation.Emulated
encapsulateStyle(
  css: string,
  componentId: string
): string
```

#### HMR Support

```typescript
// Generate HMR update module
generateHmrModule(
  componentId: string,
  templateJs: string,
  styles: string[],
  declarationsJs: string,
  constsJs: string
): string
```

##### HMR + reload behavior matrix

The Vite plugin's `handleHotUpdate` hook dispatches every file change
into one of these branches, mirroring Angular CLI's official behavior
(`@angular/build` esbuild dev server):

| File                                                                 | Change                                    | Action                                    |
| -------------------------------------------------------------------- | ----------------------------------------- | ----------------------------------------- |
| External `.html` (templateUrl)                                       | any                                       | `angular:component-update` HMR, no reload |
| External `.css/.scss/.sass/.less` (styleUrl)                         | any                                       | `angular:component-update` HMR, no reload |
| Component `.ts`                                                      | inline template only                      | `angular:component-update` HMR, no reload |
| Component `.ts`                                                      | inline `styles: [...]` only               | `angular:component-update` HMR, no reload |
| Component `.ts`                                                      | both inline template and styles           | `angular:component-update` HMR, no reload |
| Component `.ts`                                                      | class body / imports / decorator metadata | full reload                               |
| Non-component `.ts` (utils, services, constants, lazy `*.routes.ts`) | any                                       | full reload                               |
| Global stylesheet (no `styleUrl` owner)                              | any                                       | Vite default style HMR                    |
| Anything in `node_modules/` or `*.spec.ts`                           | any                                       | ignore                                    |

Set `liveReload: false` to disable both HMR and reloads — the plugin
returns from `handleHotUpdate` without sending any event.

### Transform Options

```typescript
interface TransformOptions {
  // Output
  sourcemap?: boolean

  // Compilation mode
  jit?: boolean
  hmr?: boolean
  advancedOptimizations?: boolean
  useDomOnlyMode?: boolean

  // i18n
  i18nUseExternalIds?: boolean

  // Final component style output
  minifyComponentStyles?: boolean

  // Component metadata
  selector?: string
  standalone?: boolean
  encapsulation?: 'Emulated' | 'None' | 'ShadowDom'
  changeDetection?: 'Default' | 'OnPush'
  preserveWhitespaces?: boolean

  // Cross-file elision
  crossFileElision?: boolean
  baseDir?: string
  tsconfigPath?: string
}
```

### Vite Plugin Options

```typescript
interface AngularPluginOptions {
  // Project configuration
  tsconfig?: string
  workspaceRoot?: string

  // File filtering
  include?: string | string[]
  exclude?: string | string[]

  // Features
  sourcemap?: boolean
  hmr?: boolean
  jit?: boolean
  advancedOptimizations?: boolean
  useDomOnlyMode?: boolean
  zoneless?: boolean
  liveReload?: boolean

  // Style processing
  inlineStylesExtension?: string
  minifyComponentStyles?: boolean | 'auto'

  // File replacements
  fileReplacements?: Array<{
    replace: string
    with: string
  }>
}
```

`minifyComponentStyles` resolves like this:

- `true`: always minify component styles
- `false`: never minify component styles
- `"auto"` or `undefined`: follow the resolved Vite minification settings

For `"auto"`, the plugin uses `build.cssMinify` when it is set, otherwise it falls back to `build.minify`. In dev, `"auto"` defaults to `false`.

### Library builds (`.d.ts`)

For publishing an Angular library (the ng-packagr-style workflow, e.g. with
Rolldown/tsdown), set `compilationMode: 'partial'`. This emits partial
declarations (`ɵɵngDeclareComponent`, …) in the JavaScript output, and the
plugin also augments the emitted `.d.ts` with Angular's Ivy type declarations
(`static ɵfac`, `static ɵcmp`, …) so downstream consumers get full template
type-checking against your library.

```typescript
// vite.config.ts — Angular library build
import { angular } from '@oxc-angular/vite'
import dts from 'rolldown-plugin-dts' // or vite-plugin-dts / tsdown

export default defineConfig({
  plugins: [angular({ compilationMode: 'partial' }), dts()],
  build: { lib: { entry: 'src/public-api.ts', formats: ['es'] } },
})
```

The plugin does **not** generate the base `.d.ts` itself — a declaration
generator (`rolldown-plugin-dts`, `vite-plugin-dts`, `tsdown`, or `tsc`) must
produce them. The Angular members are then spliced into those files during
`generateBundle`. The injected members reference `i0` (the `@angular/core`
namespace), and the plugin adds `import * as i0 from "@angular/core";` to any
`.d.ts` it augments.

## Vite Plugin Architecture

The Vite plugin consists of these sub-plugins:

1. **Transform Plugin** - Transforms Angular TypeScript files
2. **HMR Plugin** - Handles hot module replacement for templates and styles
3. **Styles Plugin** - Processes and encapsulates component styles
4. **Dts Plugin** - Augments library `.d.ts` with Ivy type declarations (partial mode)

### HMR Routes

| Route                | Description                      |
| -------------------- | -------------------------------- |
| `/@ng/component/:id` | Serves compiled template updates |
| `/@ng/styles/:id`    | Serves style updates             |

## Supported Angular Features

### Templates

- Control flow (@if, @for, @switch, @defer)
- Property/attribute/class/style bindings
- Event bindings and two-way binding
- Template references and variables
- Content projection (ng-content)
- Structural directives

### Components

- Standalone components
- Host bindings and listeners
- Input/Output decorators
- Query decorators (ViewChild, ContentChild, etc.)
- View encapsulation modes
- Change detection strategies

### Styles

- Inline styles
- External styleUrls
- ViewEncapsulation.Emulated
- CSS attribute selector scoping

### i18n

- External message IDs
- File-based naming

## Platform Support

Pre-built binaries for:

| Platform | Architecture            |
| -------- | ----------------------- |
| macOS    | Apple Silicon (aarch64) |
| macOS    | Intel (x86_64)          |
| Windows  | x86_64                  |
| Windows  | aarch64                 |
| Linux    | x86_64 (glibc)          |
| Linux    | x86_64 (musl)           |
| Linux    | aarch64 (glibc)         |
| Linux    | aarch64 (musl)          |

## Requirements

- Node.js 20.19.0+ or 22.12.0+
- Vite 6.0.0+ (for Vite plugin)

## Development

```bash
# Build native bindings
pnpm run build:native

# Build TypeScript
pnpm run build:ts

# Run tests
pnpm test

# Run E2E tests
pnpm run test:e2e
```

### Build Features

| Feature              | Description                         |
| -------------------- | ----------------------------------- |
| `allocator`          | Use MiMalloc for better performance |
| `cross_file_elision` | Enable cross-file import analysis   |

```bash
# Build with all features
pnpm run build-dev --features allocator,cross_file_elision --release
```

## Project Structure

```
napi/angular-compiler/
├── src/
│   └── lib.rs              # NAPI bindings (Rust)
├── core/                    # TypeScript utilities
│   ├── index.ts            # Main exports
│   ├── program.ts          # OxcNgtscProgram
│   ├── compiler.ts         # OxcAngularCompiler
│   └── config.ts           # Configuration reader
├── vite-plugin/            # Vite plugin
│   ├── index.ts            # Main plugin
│   ├── angular-jit-plugin.ts
│   └── angular-build-optimizer-plugin.ts
├── e2e/
│   └── compare/            # Comparison test runner
└── package.json
```

## Related

- [oxc](https://github.com/oxc-project/oxc) - The Oxidation Compiler
- [Angular](https://angular.dev) - Angular framework
- [@angular/compiler](https://www.npmjs.com/package/@angular/compiler) - Official Angular compiler

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

  // File replacements
  fileReplacements?: Array<{
    replace: string
    with: string
  }>
}
```

## Vite Plugin Architecture

The Vite plugin consists of three sub-plugins:

1. **Transform Plugin** - Transforms Angular TypeScript files
2. **HMR Plugin** - Handles hot module replacement for templates and styles
3. **Styles Plugin** - Processes and encapsulates component styles

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

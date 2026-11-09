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

import { angular } from '@oxc-angular/vite/vite-plugin'
import { defineConfig } from 'vite'

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
import { transformAngularFile, compileTemplateSync, extractComponentUrls } from '@oxc-angular/vite'

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
// Extract templateUrl and styleUrls. Pass the options the file is transformed
// with (`resolveImportedValues`, `baseDir`, `tsconfigPath`) so the URLs are
// evaluated the same way, imported constants included.
extractComponentUrls(
  source: string,
  filename: string,
  options?: TransformOptions
): Promise<ComponentUrls>
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

"No reload" means no reload is requested. Vite full-reloads any changed
`.html` whose module list is empty or holds no `js` module, so the plugin
registers each component template in the module graph (`addWatchFile`) and
returns it as its own HMR boundary. Without that, Vite puts a `full-reload`
on the socket on top of every template edit. Its client usually drops that
payload because the path does not match `location.pathname` — but in
`middlewareMode` the path is `*`, which always reloads
([#443](https://github.com/voidzero-dev/oxc-angular-compiler/issues/443)).

`index.html` is not a component template, so it still triggers a full reload.

### Transform Options

```typescript
interface TransformOptions {
  // Output
  sourcemap?: boolean

  // Compilation mode
  jit?: boolean
  hmr?: boolean
  advancedOptimizations?: boolean

  // i18n (note the capital N, as the N-API bindings generate it)
  i18NUseExternalIds?: boolean
  enableI18nLegacyMessageIdFormat?: boolean

  // Final component style output
  minifyComponentStyles?: boolean

  // Component metadata
  selector?: string
  standalone?: boolean
  encapsulation?: 'Emulated' | 'None' | 'ShadowDom'
  changeDetection?: 'Default' | 'OnPush'
  // Default for components that do not set their own (project-wide option)
  preserveWhitespaces?: boolean

  // Angular compiler options of the same name (default: false / unset)
  forbidOrphanComponents?: boolean
  enableTemplateSourceLocations?: boolean // needs rootDirs; full mode only
  // Roots that ɵsetClassDebugInfo's filePath and source locations are relative to
  rootDirs?: string[]
  // ts.sys.useCaseSensitiveFileNames; false lowercases those paths, like ngtsc
  // (default: true)
  useCaseSensitiveFileNames?: boolean

  // Cross-file elision
  crossFileElision?: boolean
  // Evaluate decorator metadata imported from other files (as ngtsc does)
  resolveImportedValues?: boolean
  baseDir?: string
  tsconfigPath?: string
}
```

### Vite Plugin Options

```typescript
interface PluginOptions {
  // Project configuration
  tsconfig?: string
  workspaceRoot?: string
  // Application's Angular version, used for app code and linked libraries
  // (default: the installed @angular/core version, else latest)
  angularVersion?: { major: number; minor: number; patch: number }

  // Features
  jit?: boolean
  liveReload?: boolean
  // Default: zoneless unless zone.js is installed (see below)
  zoneless?: boolean
  sourceMap?: boolean | { scripts?: boolean; vendor?: boolean }

  // Style processing
  inlineStylesExtension?: string
  minifyComponentStyles?: boolean | 'auto'

  // File replacements
  fileReplacements?: Array<{
    replace: string
    with: string
  }>

  // SSR
  ssrEntry?: string

  // Template hook, applied during both the initial build and HMR
  templateTransform?: (content: string, filePath: string) => string

  // Project-wide default (`angularCompilerOptions.preserveWhitespaces`);
  // a component's own `preserveWhitespaces` wins (default: false)
  preserveWhitespaces?: boolean

  // Angular compiler options of the same name (default: false)
  forbidOrphanComponents?: boolean
  enableTemplateSourceLocations?: boolean
  // Roots for ɵsetClassDebugInfo's filePath and source locations,
  // resolved against workspaceRoot (default: [workspaceRoot], like ngtsc)
  rootDirs?: string[]
  // false lowercases those paths, like ngtsc on macOS and Windows
  // (default: checked on the project's file system, like ts.sys)
  caseSensitiveFileNames?: boolean
}
```

`minifyComponentStyles` resolves like this:

- `true`: always minify component styles
- `false`: never minify component styles
- `"auto"` or `undefined`: follow the resolved Vite minification settings

For `"auto"`, the plugin uses `build.cssMinify` when it is set, otherwise it falls back to `build.minify`. In dev, `"auto"` defaults to `false`.

### zone.js and `async`/`await`

zone.js cannot track native `await`. Unless the app is zoneless, the plugin
downlevels `async` functions, async generators and `for await` in application code
to generators, as the Angular CLI does for apps with `zone.js` in their
polyfills, so `fakeAsync` with `tick()`/`flushMicrotasks()` drives code that
awaits, and change detection runs after an `await`. The helpers are imported
from `@oxc-project/runtime`, which Vite resolves. Server (SSR) transforms and
`node_modules` are left as they are. Vitest's `node` environment transforms as
SSR, so run zone.js specs in `jsdom`, `happy-dom` or the browser.

Without `zoneless`, an app is taken to be zoneless when no `node_modules` above
the Vite root contains `zone.js`. Set `zoneless: true` to keep `async` native in a
zoneless app that still has zone.js installed, or `zoneless: false` to force the
downleveling.

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
import { dts } from 'rolldown-plugin-dts'

export default defineConfig({
  plugins: [angular({ compilationMode: 'partial' }), dts()],
  build: { lib: { entry: 'src/public-api.ts', formats: ['es'] } },
})
```

The plugin does **not** generate the base `.d.ts` itself — a declaration
generator must write them. Like ngtsc, the plugin then adds to each
declaration file only the members of its own source file (two classes with
the same name in different files each get their own), adds the namespace
imports the members use (`import * as i0 from "@angular/core";`, …), and
keeps the file's declaration map in step. What is covered:

| Generator                                                | How                                                                          |
| -------------------------------------------------------- | ---------------------------------------------------------------------------- |
| `rolldown-plugin-dts`, tsdown `dts` (`esm` and `cjs`)    | each `.d.ts` module as it loads, before bundling                             |
| `.d.ts` assets (`@rollup/plugin-typescript`, `emitFile`) | in `generateBundle`, matched to their source by path, as `tsc` lays them out |
| `vite-plugin-dts` with its default `outDir`              | the files it wrote under the build's output dir, matched by path             |

Not covered:

- declarations written outside the build, such as a `tsc` run of its own, or
  `vite-plugin-dts` `outDirs` outside `build.outDir`;
- `vite-plugin-dts` `rollupTypes` bundles, which only get members when their
  path matches one source file;
- the `.d.cts` of tsdown when the plugin comes through `fromVite` (such
  plugins don't get tsdown's `tsdownConfig` hook, which that pass needs).

Files written by another generator (`vite-plugin-dts`) are patched after they
are written, so a tool that reads them earlier in the same build sees them
without the members.

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

- Legacy message IDs: `enableI18nLegacyMessageIdFormat: true` (also a Vite plugin option) makes
  `$localize` messages carry them, for translation files keyed by legacy IDs. Off by default, as
  in the `tsconfig.json` that `ng new` generates.

Not supported: `i18nNormalizeLineEndingsInICUs`.

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

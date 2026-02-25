# OXC Angular Compiler

A high-performance Angular template compiler written in Rust, leveraging the [Oxc](https://github.com/oxc-project/oxc) infrastructure for blazing-fast compilation.

## Features

- **Rust-Powered Performance** - Native compilation via NAPI-RS for maximum speed
- **Vite Integration** - First-class Vite plugin with full HMR support
- **Drop-in Compatible** - API compatible with `@angular/compiler-cli`
- **Full Angular Support** - Components, directives, pipes, injectables, and NgModules
- **Hot Module Replacement** - Fast refresh for templates and styles during development
- **Style Encapsulation** - ViewEncapsulation.Emulated, None, and ShadowDom support
- **i18n Ready** - Message extraction with XLIFF 1.2/2.0, XMB, and XTB formats
- **Build Optimizations** - Constant folding, pure function extraction, unused code removal

## Installation

```bash
npm install @oxc-angular/vite
# or
pnpm add @oxc-angular/vite
# or
yarn add @oxc-angular/vite
```

## Quick Start

### Vite Plugin

```typescript
// vite.config.ts
import { defineConfig } from 'vite'
import { angular } from '@oxc-angular/vite'

export default defineConfig({
  plugins: [
    angular({
      // Optional configuration
      angularVersion: 21,
      enableHmr: true,
    }),
  ],
})
```

### Programmatic API

```typescript
import { compileTemplate, transformAngularFile } from '@oxc-angular/vite/api'

// Compile a template string
const result = await compileTemplate(
  '<div>{{ message }}</div>',
  'AppComponent',
  'app.component.ts',
  {
    angularVersion: 21,
    enableHmr: false,
  },
)

console.log(result.code)

// Transform an entire Angular file
const transformed = await transformAngularFile(
  `
  import { Component } from '@angular/core';

  @Component({
    selector: 'app-root',
    template: '<h1>Hello {{ name }}</h1>',
  })
  export class AppComponent {
    name = 'World';
  }
  `,
  'app.component.ts',
)

console.log(transformed.code)
```

## API Reference

### `compileTemplate(template, componentName, filePath, options?)`

Compiles an Angular template string to JavaScript.

| Parameter       | Type               | Description                  |
| --------------- | ------------------ | ---------------------------- |
| `template`      | `string`           | The HTML template string     |
| `componentName` | `string`           | Name of the component class  |
| `filePath`      | `string`           | Path to the component file   |
| `options`       | `TransformOptions` | Optional compilation options |

Returns: `Promise<TemplateCompileResult>`

### `transformAngularFile(source, filename, options?)`

Transforms an Angular TypeScript file, compiling any component templates.

| Parameter  | Type               | Description                  |
| ---------- | ------------------ | ---------------------------- |
| `source`   | `string`           | The TypeScript source code   |
| `filename` | `string`           | Path to the source file      |
| `options`  | `TransformOptions` | Optional compilation options |

Returns: `Promise<TransformResult>`

### Transform Options

```typescript
interface TransformOptions {
  // Angular version (19, 20, 21)
  angularVersion?: number

  // Enable Hot Module Replacement
  enableHmr?: boolean

  // Enable cross-file type elision
  enableCrossFileElision?: boolean

  // i18n configuration
  i18nNormalizeLineEndingsInIcus?: boolean
  i18nUseExternalIds?: boolean

  // Style encapsulation mode
  // 'emulated' | 'none' | 'shadow-dom'
  encapsulation?: string
}
```

## Architecture

The compiler implements a 6-stage pipeline:

```
HTML Template
      ↓
   PARSING        → HTML AST
      ↓
  TRANSFORM       → R3 AST (Angular's internal representation)
      ↓
  INGESTION       → Intermediate Representation (IR)
      ↓
TRANSFORMATION    → 67 optimization phases
      ↓
   EMISSION       → Output AST
      ↓
CODE GENERATION   → JavaScript
```

### Core Modules

| Module      | Purpose                                |
| ----------- | -------------------------------------- |
| `parser`    | HTML and expression parsing            |
| `transform` | HTML to R3 AST transformation          |
| `ir`        | Intermediate Representation operations |
| `pipeline`  | 67 transformation phases               |
| `output`    | JavaScript code generation             |
| `hmr`       | Hot Module Replacement support         |
| `styles`    | CSS processing and encapsulation       |
| `i18n`      | Internationalization support           |

## Platform Support

Pre-built binaries are available for:

| Platform | Architecture                                |
| -------- | ------------------------------------------- |
| macOS    | Apple Silicon (aarch64), Intel (x86_64)     |
| Windows  | x86_64, aarch64                             |
| Linux    | x86_64 (glibc, musl), aarch64 (glibc, musl) |

### Requirements

- Node.js 20.19.0+ or 22.12.0+
- Vite 6.0.0+ (for Vite plugin)

## Development

### Prerequisites

- Rust 1.90.0+
- Node.js 20.19.0+
- pnpm

### Setup

```bash
# Clone the repository
git clone https://github.com/voidzero-dev/oxc-angular-compiler.git
cd oxc-angular-compiler

# Install dependencies
pnpm install

# Initialize submodules
git submodule update --init

# Build NAPI bindings
pnpm build

# Run tests
pnpm test
```

### Development Commands

```bash
# Build (development)
pnpm build-dev

# Run Rust tests
cargo test

# Run Node.js tests
pnpm test

# Run E2E tests
pnpm test:e2e

# Run conformance tests against Angular
cargo run -p oxc_angular_conformance

# Start playground
pnpm playground
```

### Project Structure

```
oxc-angular-compiler/
├── crates/
│   ├── oxc_angular_compiler/     # Core Rust compiler
│   └── angular_conformance/      # Conformance testing
├── napi/
│   └── angular-compiler/
│       ├── src/                  # NAPI bindings
│       ├── vite-plugin/          # Vite plugin (TypeScript)
│       ├── e2e/                  # E2E tests
│       └── test/                 # Integration tests
└── Cargo.toml                    # Rust workspace
```

## Benchmarks

The compiler achieves significant performance improvements over the official TypeScript-based Angular compiler. Benchmarks are available in `napi/angular-compiler/benchmarks/`.

## Contributing

Contributions are welcome! Please read the contributing guidelines before submitting pull requests.

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/amazing-feature`)
3. Commit your changes (`git commit -m 'Add amazing feature'`)
4. Push to the branch (`git push origin feature/amazing-feature`)
5. Open a Pull Request

## Related Projects

- [Oxc](https://github.com/oxc-project/oxc) - The JavaScript Oxidation Compiler
- [Angular](https://github.com/angular/angular) - The Angular framework
- [Vite](https://github.com/vitejs/vite) - Next generation frontend tooling

## Acknowledgments

- The [Oxc](https://github.com/oxc-project/oxc) team for the excellent Rust infrastructure
- The [Angular](https://github.com/angular/angular) team for the original compiler architecture

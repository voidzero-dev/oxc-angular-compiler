# Angular Compiler Comparison Tool

A testing tool that validates the OXC Angular compiler by comparing its output against the official Angular compiler.

> **Note**: This is a development/testing tool within the `@oxc-angular/vite` package. For the main package documentation, see the [parent README](../../README.md).

## Purpose

This tool ensures that the OXC Angular compiler produces **semantically equivalent** output to Angular's official compiler, enabling safe adoption for production use.

### Key Features

- **Two comparison modes**: Template-only (fast) and full-file (comprehensive)
- **80+ test fixtures** across 25 categories
- **Real-world project testing** with presets for Bitwarden, ClickUp, Material-Angular
- **Semantic comparison** with automatic normalization of known differences
- **Detailed diff reporting** at function and line level

## Quick Start

```bash
# Install dependencies
pnpm install

# Run fixture tests
pnpm compare --fixtures

# Run full-file comparison on a project
pnpm compare -p /path/to/angular-project --full-file

# Use a preset
pnpm compare --preset bitwarden --full-file
```

## Comparison Modes

### Template-Only Mode (Default)

Compares template compilation output only. Faster, suitable for rapid iteration.

```bash
pnpm compare -p /path/to/project
```

### Full-File Mode

Compares complete `.js` file output including imports, class definitions, and metadata.

```bash
pnpm compare -p /path/to/project --full-file
```

## Available Presets

| Preset             | Description               | Components |
| ------------------ | ------------------------- | ---------- |
| `bitwarden`        | Bitwarden clients project | ~670       |
| `material-angular` | Angular Material/CDK      | ~635       |
| `clickup`          | ClickUp Frontend (full)   | ~5,860     |
| `clickup-core`     | ClickUp core libraries    | ~350       |

```bash
pnpm compare --preset bitwarden --full-file
pnpm compare --preset material-angular --full-file
pnpm compare --preset clickup --full-file  # Requires 16GB+ heap
```

## CLI Options

```bash
pnpm compare [options]

Options:
  -p, --project <path>     Path to Angular project
  --preset <name>          Use predefined configuration
  --full-file              Enable full-file comparison mode
  --fixtures               Run fixture tests only
  --category <name>        Filter fixtures by category (repeatable)
  --detailed-diff          Show detailed line-by-line diffs
  --validate-extraction    Cross-validate metadata extraction
  --list-presets           List available presets
  --list-fixtures          List available fixtures
  --help                   Show help
```

## Project Structure

```
e2e/compare/
├── src/
│   ├── index.ts              # CLI entry point
│   ├── runner.ts             # Main orchestrator
│   ├── compare.ts            # Semantic comparison logic
│   ├── presets.ts            # Preset configurations
│   ├── types.ts              # Type definitions
│   ├── compilers/
│   │   ├── oxc.ts            # OXC compiler wrapper
│   │   ├── angular.ts        # Angular template compiler
│   │   └── angular-ngtsc.ts  # Angular NgtscProgram (full-file)
│   └── discovery/
│       ├── finder.ts         # Component discovery
│       └── typescript-extractor.ts
├── fixtures/                  # 80+ test fixtures
│   ├── {category}/*.fixture.ts
│   ├── index.ts
│   ├── types.ts
│   └── runner.ts
└── package.json
```

## Comparison Workflow

```
1. Discover components (glob patterns, metadata extraction)
2. Compile with OXC (NAPI bindings)
3. Compile with Angular (NgtscProgram or template compiler)
4. Normalize both outputs:
   ├── Format with oxfmt
   ├── Normalize PURE comments
   ├── Normalize template literals
   ├── Normalize nullish coalescing
   ├── Remap const references (_c0, _c1, etc.)
   └── Normalize @Inject parameter types
5. Compare normalized outputs
6. For mismatches: Extract function-level diffs
```

## Semantic Comparison

The framework handles known differences between compilers:

### Handled Automatically

| Difference             | Description                                    |
| ---------------------- | ---------------------------------------------- |
| **Formatting**         | Whitespace, indentation normalized via `oxfmt` |
| **Const ordering**     | Const pool indices remapped by value           |
| **Variable naming**    | `_r45` vs `_r46` counters (scoped, equivalent) |
| **PURE comments**      | `/* @__PURE__ */` spacing normalized           |
| **Nullish coalescing** | `a ?? b` transpilation normalized              |
| **Parentheses**        | Extra parens removed where equivalent          |

### Known Differences (Not Bugs)

| Difference                   | Description                                                   |
| ---------------------------- | ------------------------------------------------------------- |
| **Barrel exports**           | OXC resolves to source files, Angular keeps barrel paths      |
| **Class field declarations** | Upstream `oxc-transform` issue with `useDefineForClassFields` |
| **setClassMetadata**         | OXC doesn't emit (DevTools feature)                           |

## Test Fixtures

### Categories

| Category         | Description                                |
| ---------------- | ------------------------------------------ |
| `animations`     | @angular/animations support                |
| `bindings`       | Property, class, style, attribute bindings |
| `control-flow`   | @if, @for, @switch blocks                  |
| `defer`          | @defer blocks with triggers                |
| `host-bindings`  | Host binding/listener compilation          |
| `i18n`           | Internationalization                       |
| `inputs-outputs` | @Input/@Output decorators                  |
| `pipes`          | Pipe usage in templates                    |
| `providers`      | Providers and viewProviders                |
| `regressions`    | Past bug fixes                             |
| `styles`         | Encapsulation, inline/external             |
| `templates`      | Content projection, ng-template            |

### Running Fixtures

```bash
# All fixtures
pnpm compare --fixtures

# Specific categories
pnpm compare --fixtures --category defer
pnpm compare --fixtures --category control-flow --category animations
```

## Output Format

### Console Output

```
File-to-File Comparison Results
-------------------------------
Files compared:     669
Files matched:      630 (94.2%)
Files mismatched:   39
Oxc errors:         0
Angular errors:     0

Mismatched Files:
  - /path/to/component.ts
    Diff: 2 import diff(s), 1 func diff(s)
```

### JSON Report

Written to `compare-report.json`:

```typescript
{
  summary: { total, matched, mismatched, passRate },
  metadata: { generatedAt, projectRoot, durationMs },
  fileResults: [{
    filePath: string,
    status: "match" | "mismatch" | "oxc-error" | "ts-error",
    comparisonDetails: {
      importDiffs?: ImportDiff[],
      functionDiffs?: FunctionDiff[],
      classDiffs?: ClassDiff[]
    }
  }]
}
```

## Current Match Rates

| Project          | Match Rate | Notes                    |
| ---------------- | ---------- | ------------------------ |
| Bitwarden        | 94.2%      | Barrel export resolution |
| Material-Angular | 99.2%      | Class field declarations |
| ClickUp          | 95.9%      | Various edge cases       |

## Architecture

### NAPI Bindings

The tool uses OXC via NAPI-RS bindings:

```typescript
// Metadata extraction
extractComponentMetadataSync(source, filePath)

// Template compilation
compileTemplateSync(template, className, filePath, metadata)

// Full file transformation
transformAngularFile(sourceCode, filePath, options)
```

### Angular APIs

```typescript
// Template parsing
parseTemplate(template, sourceMapUrl, interpolationConfig)

// Component compilation
compileComponentFromMetadata(metadata)
```

## Performance

- **Parallel component discovery**: Up to 2x CPU cores
- **Batch OXC compilation**: All files in single call
- **Progress indicators**: Every 50-100 files
- **Memory**: ClickUp requires `NODE_OPTIONS="--max-old-space-size=16384"`

## Troubleshooting

### Out of Memory

For large projects like ClickUp:

```bash
NODE_OPTIONS="--max-old-space-size=16384" pnpm compare --preset clickup --full-file
```

### Module Resolution Errors

Full-file mode requires valid `tsconfig.json`. Falls back to template-only if resolution fails.

### Debugging Mismatches

1. Check `compare-report.json` for detailed diffs
2. Use `--detailed-diff` for line-by-line output
3. Check `comparisonDetails.importDiffs` for import issues
4. Check `comparisonDetails.functionDiffs` for template issues

## Contributing

### Adding Fixtures

1. Create `fixtures/{category}/{name}.fixture.ts`
2. Export fixture object with required fields
3. Run `pnpm compare --fixtures --category {category}`

### Fixture Structure

```typescript
export const myFixture: Fixture = {
  name: 'my-fixture',
  category: 'bindings',
  description: 'Tests property binding',
  template: `<div [class.active]="isActive"></div>`,
  className: 'MyComponent',
}
```

## License

MIT

# Bitwarden Web Vault - OXC Angular Benchmark

This benchmark compares the build performance of the OXC Angular Compiler (via Vite) against Angular's `@ngtools/webpack` (bitwarden's existing webpack build).

## Prerequisites

1. **Clone bitwarden-clients** repository as a sibling to the oxc repository:

   ```bash
   cd ~/workspace/github  # or wherever your repos are
   git clone https://github.com/bitwarden/clients.git bitwarden-clients
   ```

2. **Install bitwarden dependencies**:

   ```bash
   cd bitwarden-clients
   pnpm install
   ```

3. **Install benchmark dependencies**:
   ```bash
   cd oxc/napi/angular-compiler/benchmarks/bitwarden
   pnpm install
   ```

## Usage

### Run Full Benchmark

Runs both cold build and incremental build comparisons:

```bash
pnpm benchmark
```

### Run Specific Benchmarks

```bash
# Only cold build comparison
pnpm benchmark --cold

# Only incremental build comparison
pnpm benchmark --incremental

# Only test Vite/OXC build (skip webpack)
pnpm benchmark --vite-only

# Only test Webpack build (skip Vite)
pnpm benchmark --webpack-only

# Custom number of iterations
pnpm benchmark --iterations=5

# Verbose output
pnpm benchmark --verbose
```

### Development Server

Start the Vite dev server for interactive testing:

```bash
pnpm dev
```

### Production Build

Build with Vite/OXC only:

```bash
pnpm build
```

## Configuration

### Path Customization

If your bitwarden-clients repository is in a different location, update the `BITWARDEN_ROOT` path in:

- `vite.config.ts`
- `benchmark.ts`
- `postcss.config.cjs`
- `tailwind.config.cjs`
- `tsconfig.app.json`

### Vite Configuration

The `vite.config.ts` includes:

- **vite-tsconfig-paths**: Resolves 70+ path aliases from bitwarden's tsconfig
- **@oxc-angular/vite**: OXC's Angular compiler plugin
- **SCSS preprocessing**: With proper include paths for bitwarden's styles
- **PostCSS**: Matching bitwarden's configuration (tailwind, autoprefixer, etc.)
- **process.env polyfills**: For bitwarden's environment variable usage

## What's Being Compared

| Aspect           | Vite/OXC            | Webpack/@ngtools              |
| ---------------- | ------------------- | ----------------------------- |
| Bundler          | Vite (Rolldown)     | Webpack                       |
| Angular Compiler | OXC (Rust)          | @ngtools/webpack (TypeScript) |
| CSS Processing   | Vite's CSS pipeline | MiniCssExtractPlugin          |
| Source Maps      | Vite native         | Webpack devtool               |

## Expected Results

Based on initial testing, OXC's Rust-based Angular compiler typically provides:

- **Cold builds**: 2-5x faster than webpack
- **Incremental builds**: 5-10x faster than webpack
- **Output size**: Comparable (may vary based on tree-shaking differences)

## Troubleshooting

### "bitwarden-clients not found"

Ensure the bitwarden-clients repository is cloned to the expected location (sibling to oxc directory).

### "Failed to resolve import"

Path aliases may need adjustment. Check that:

1. bitwarden's `tsconfig.base.json` is accessible
2. vite-tsconfig-paths plugin is configured correctly

### "Cannot find module '@angular/...'"

Run `pnpm install` in both the benchmark directory and bitwarden-clients.

### Style/CSS errors

SCSS include paths may need adjustment based on bitwarden's structure.

import { defineConfig } from 'vite-plus'

// `vp run compare` runs the compare harness's fixture tests in two tasks, so that the slow part,
// compiling every fixture with Angular's compiler, is cached.
const compareDir = 'napi/angular-compiler/e2e/compare'
const node = 'node --import @oxc-node/core/register'

export default defineConfig({
  run: {
    tasks: {
      // The fixtures' ngtsc reference outputs. The script loads only the Angular side of the
      // harness, so its inputs are the fixtures, that code and the installed packages, not Oxc.
      'ng-baseline': {
        command: `${node} scripts/generate-ng-baseline.ts --save-ng-baseline ng-baseline-fixtures.json`,
        cwd: compareDir,
      },
      // Compiles the fixtures with Oxc and compares the output with the baseline. Not cached:
      // it reads the binding, which `pnpm build-dev` builds outside of `vp run`.
      compare: {
        command: `${node} src/index.ts --fixtures --ng-baseline ng-baseline-fixtures.json`,
        cwd: compareDir,
        dependsOn: ['ng-baseline'],
        cache: false,
      },
    },
  },
})

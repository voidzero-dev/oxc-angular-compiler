import { existsSync, realpathSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

/**
 * Whether the file system of the project at `root` tells `App.ts` from `app.ts`,
 * the default of the `caseSensitiveFileNames` option.
 *
 * Checked like TypeScript's `ts.sys.useCaseSensitiveFileNames`, which ngtsc
 * uses: `false` on Windows, else whether a known file is missing once the case
 * of its path is swapped. TypeScript checks its own `typescript.js`, so this
 * checks the project's TypeScript install (on the same volume as the project),
 * or this plugin's own file when there is none.
 */
export function isFileSystemCaseSensitive(root: string): boolean {
  if (process.platform === 'win32') return false
  const file = findTypeScript(resolve(root)) ?? fileURLToPath(import.meta.url)
  let real: string
  try {
    real = realpathSync(file)
  } catch {
    return true
  }
  return !existsSync(swapCase(real))
}

/** TypeScript's `swapCase`: each `\w` character in the other case. */
export function swapCase(path: string): string {
  return path.replace(/\w/g, (ch) => {
    const up = ch.toUpperCase()
    return ch === up ? ch.toLowerCase() : up
  })
}

/** `typescript/lib/typescript.js` in the `node_modules` of `root` or above. */
function findTypeScript(root: string): string | undefined {
  if (process.versions['pnp']) {
    try {
      return createRequire(join(root, 'package.json')).resolve('typescript')
    } catch {
      return undefined
    }
  }
  for (let dir = root; ; dir = dirname(dir)) {
    const file = join(dir, 'node_modules/typescript/lib/typescript.js')
    if (existsSync(file)) return file
    if (dirname(dir) === dir) return undefined
  }
}

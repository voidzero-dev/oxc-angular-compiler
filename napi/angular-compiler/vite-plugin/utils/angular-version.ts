import { existsSync, readFileSync } from 'node:fs'
import { createRequire } from 'node:module'
import { dirname, join } from 'node:path'

import type { AngularVersion } from '#binding'

/**
 * The version of `@angular/core` installed for the project at `root`, the
 * default of the `angularVersion` option. `undefined` when it isn't found or
 * is a build of Angular's own repository (`0.0.0-...`): the compiler then
 * targets the latest version.
 */
export function installedAngularVersion(root: string): AngularVersion | undefined {
  const file = findAngularCorePackageJson(root)
  if (!file) return undefined
  try {
    return parseAngularVersion(JSON.parse(readFileSync(file, 'utf8')).version)
  } catch {
    return undefined
  }
}

/** `22.1.0-next.3` as an `AngularVersion`; build metadata (`+...`) is dropped. */
export function parseAngularVersion(version: unknown): AngularVersion | undefined {
  if (typeof version !== 'string') return undefined
  const match = /^(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+.*)?$/.exec(version.trim())
  if (!match) return undefined
  const [, major, minor, patch, prerelease] = match
  if (Number(major) === 0) return undefined
  return {
    major: Number(major),
    minor: Number(minor),
    patch: Number(patch),
    ...(prerelease ? { prerelease } : {}),
  }
}

/**
 * Looked up in `node_modules` rather than with `require.resolve`, which also
 * follows `NODE_PATH` (set by pnpm to its hoisted store), like zone.js in
 * `angular-async-downlevel-plugin.ts`.
 */
function findAngularCorePackageJson(root: string): string | undefined {
  if (process.versions['pnp']) {
    try {
      return createRequire(join(root, 'package.json')).resolve('@angular/core/package.json')
    } catch {
      return undefined
    }
  }
  for (let dir = root; ; dir = dirname(dir)) {
    const file = join(dir, 'node_modules/@angular/core/package.json')
    if (existsSync(file)) return file
    if (dirname(dir) === dir) return undefined
  }
}

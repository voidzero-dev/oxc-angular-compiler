import { describe, expect, it } from 'vitest'

import { angular, type PluginOptions } from '../vite-plugin/index.js'

const SOURCE = `import { Component } from '@angular/core';

@Component({ selector: 'app-root', template: '<div>hi</div>' })
export class AppComponent {}
`

/** The plugin's output for `id`, after Vite resolved its config with `root`. */
async function transform(options: PluginOptions, root: string, id: string): Promise<string> {
  const plugin = angular(options).find((candidate) => candidate.name === '@oxc-angular/vite')
  if (!plugin?.transform || typeof plugin.transform === 'function') {
    throw new Error('Expected plugin transform handler')
  }
  const configResolved = plugin.configResolved as unknown as
    | { handler: (config: unknown) => void }
    | ((config: unknown) => void)
  const config = { root, build: {}, isProduction: false }
  if (typeof configResolved === 'function') configResolved(config)
  else configResolved?.handler(config)

  const result = await plugin.transform.handler.call(
    {
      error(message: string) {
        throw new Error(message)
      },
      warn() {},
    } as any,
    SOURCE,
    id,
  )
  if (!result || typeof result !== 'object' || !('code' in result)) {
    throw new Error('Expected transform result with code')
  }
  return (result.code as string).replace(/\s+/g, '')
}

describe('@oxc-angular/vite project-relative paths', () => {
  it('makes ɵsetClassDebugInfo filePath relative to the workspace root', async () => {
    const code = await transform({ workspaceRoot: '/proj' }, '/proj', '/proj/src/app.component.ts')
    expect(code).toContain('filePath:"src/app.component.ts"')
    expect(code).not.toContain('attachSourceLocations')
  })

  it('keeps filePath for a library outside the app root of a monorepo', async () => {
    // ngtsc's root is the workspace (its working directory), not the app.
    const code = await transform(
      { workspaceRoot: '/mono' },
      '/mono/apps/app',
      '/mono/libs/ui/app.component.ts',
    )
    expect(code).toContain('filePath:"libs/ui/app.component.ts",lineNumber:4')
  })

  it('uses the rootDirs option over the workspace root', async () => {
    const code = await transform(
      { workspaceRoot: '/proj', rootDirs: ['/proj/src'] },
      '/proj',
      '/proj/src/app.component.ts',
    )
    expect(code).toContain('filePath:"app.component.ts"')
  })

  it('emits ɵɵattachSourceLocations with enableTemplateSourceLocations', async () => {
    // As ngtsc 22.2.3 emits it for the same file, with the project directory as its root.
    const code = await transform(
      { enableTemplateSourceLocations: true, workspaceRoot: '/proj' },
      '/proj',
      '/proj/src/app.component.ts',
    )
    expect(code).toContain('ɵɵattachSourceLocations("src/app.component.ts",[[0,90,2,46]]);')
  })
})

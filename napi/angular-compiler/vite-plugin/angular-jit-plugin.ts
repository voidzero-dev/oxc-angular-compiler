import { dirname, resolve } from 'node:path'

import { normalizePath, type Plugin, preprocessCSS, type ResolvedConfig } from 'vite'

export function jitPlugin({ inlineStylesExtension }: { inlineStylesExtension: string }): Plugin {
  let config: ResolvedConfig

  return {
    name: '@oxc-angular/vite-jit',
    configResolved(_config) {
      config = _config
    },
    resolveId: {
      filter: {
        id: /^(virtual:angular|angular:jit:)/,
      },
      handler(id, importer) {
        if (id.startsWith('angular:jit:')) {
          const path = id.split(';')[1]
          return `${normalizePath(resolve(dirname(importer as string), path))}?raw`
        }

        // Otherwise starts with virtual:angular
        return `\0${id}`
      },
    },
    load: {
      filter: {
        id: /virtual:angular:jit:style:inline;/,
      },
      async handler(id: string) {
        const styleId = id.split('style:inline;')[1]

        const decodedStyles = Buffer.from(decodeURIComponent(styleId), 'base64').toString()

        let styles: string | undefined = ''

        try {
          const compiled = await preprocessCSS(
            decodedStyles,
            `${styleId}.${inlineStylesExtension}?direct`,
            config,
          )
          styles = compiled?.code
        } catch (e) {
          console.error(e)
        }

        return `export default \`${styles}\``
      },
    },
  }
}

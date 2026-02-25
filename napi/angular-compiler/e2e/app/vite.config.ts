import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { angular } from '@oxc-angular/vite'
import { defineConfig } from 'vite'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const tsconfig = path.resolve(__dirname, './tsconfig.json')

export default defineConfig({
  plugins: [
    angular({
      tsconfig,
      liveReload: true,
    }),
  ],
})

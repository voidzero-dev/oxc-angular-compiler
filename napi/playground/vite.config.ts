import path from 'node:path'
import { fileURLToPath } from 'node:url'

// Use our local vite-plugin implementation
import { angular } from '@oxc-angular/vite'
import tailwindcss from '@tailwindcss/vite'
import { defineConfig } from 'vite'

const __filename = fileURLToPath(import.meta.url)
const __dirname = path.dirname(__filename)
const tsconfig = path.resolve(__dirname, './tsconfig.app.json')

export default defineConfig({
  plugins: [
    tailwindcss(),
    angular({
      tsconfig,
      liveReload: true,
    }),
  ],
})

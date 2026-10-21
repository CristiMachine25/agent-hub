import { resolve } from 'node:path'
import { defineConfig } from 'vitest/config'
export default defineConfig({
  resolve: { alias: { '@': resolve(import.meta.dirname, 'src'), '@shared': resolve(import.meta.dirname, 'src/shared') } },
  test: { include: ['src/**/*.test.ts', 'src/**/*.test.tsx'], environment: 'node' }
})

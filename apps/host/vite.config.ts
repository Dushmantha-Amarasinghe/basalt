/// <reference types="vitest/config" />
import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
  clearScreen: false,
  server: {
    // One above the client's, so both dev servers can run at once.
    port: 1421,
    strictPort: true,
  },
  build: {
    // WebView2 on Windows is Chromium, so there is no need to ship syntax
    // transforms for older engines.
    target: 'chrome110',
    sourcemap: true,
  },
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
  },
})

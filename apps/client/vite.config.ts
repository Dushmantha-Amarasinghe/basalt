import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react'
import path from 'node:path'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': path.resolve(__dirname, './src') },
  },
  // Tauri expects a fixed port and fails rather than silently moving.
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // WebView2 on Windows is Chromium, so there is no need to ship syntax
    // transforms for older engines.
    target: 'chrome110',
    sourcemap: true,
  },
})

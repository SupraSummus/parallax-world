import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react-swc'
import { resolve } from 'path'
import { configDefaults, defineConfig } from 'vitest/config'

export default defineConfig({
  // Relative asset URLs, so the build works under any path
  // (GitHub Pages serves it from /<repo-name>/).
  base: './',
  // The WebAssembly of bake/ (bake workers, coverage), inlined (?inline) to compile synchronously.
  assetsInclude: ['**/*.wasm'],
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      '@': resolve(import.meta.dirname, 'src'),
    },
  },
  build: {
    // three.js (~540 kB) is only loaded by the viewer page.
    chunkSizeWarningLimit: 600,
    rolldownOptions: {
      input: {
        main: resolve(import.meta.dirname, 'index.html'),
        viewer: resolve(import.meta.dirname, 'viewer.html'),
      },
    },
  },
  test: {
    // Baking whole views of FCC worlds takes a few seconds.
    testTimeout: 20_000,
    // Agents' git worktrees hold copies of the tests.
    exclude: [...configDefaults.exclude, '.claude/**'],
  },
})

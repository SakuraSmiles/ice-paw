import { defineConfig } from 'vite'
import { resolve } from 'path'

export default defineConfig({
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      name: 'IcePawUI',
      formats: ['es'],
      fileName: () => 'index.js',
    },
    cssCodeSplit: true,
    emptyOutDir: true,
  },
})

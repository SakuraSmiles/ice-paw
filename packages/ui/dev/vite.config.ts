import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'

// 预览站独立 dev server
export default defineConfig({
  root: __dirname,
  plugins: [vue()],

  resolve: {
    alias: {
      // 预览站直接消费 ui 源码，支持 HMR
      '@ice-paw/ui': resolve(__dirname, '../src/index.ts'),
      '@ice-paw/ui/styles': resolve(__dirname, '../src/styles/index.css'),
      '@': resolve(__dirname, '../src'),
    },
  },

  server: {
    port: 5173,
    strictPort: true,
    open: false,
  },
})

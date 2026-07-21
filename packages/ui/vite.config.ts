import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { resolve } from 'path'

export default defineConfig({
  plugins: [
    vue({
      template: {
        compilerOptions: {
          // cropperjs v2 使用 Web Components（<cropper-canvas> 等），
          // Vue 模板编译器遇到这些标签会尝试解析为 Vue 组件导致运行时失败
          isCustomElement: (tag) => tag.startsWith('cropper-'),
        },
      },
    }),
  ],

  // lib 模式构建
  build: {
    lib: {
      entry: resolve(__dirname, 'src/index.ts'),
      name: 'IcePawUI',
      formats: ['es', 'cjs'],
      fileName: (format) => (format === 'es' ? 'index.js' : 'index.cjs'),
    },
    rollupOptions: {
      // Vue 不打包进 lib，作为 external（避免双 Vue 实例）
      external: ['vue'],
      output: {
        // 保持 import 'vue' 不变
        globals: {
          vue: 'Vue',
        },
      },
    },
    cssCodeSplit: true,
    // sourcemap
    sourcemap: true,
    // 清理 dist
    emptyOutDir: true,
  },

  // 解析别名
  resolve: {
    alias: {
      '@': resolve(__dirname, 'src'),
    },
  },

  // 注入构建时常量
  define: {
    __VERSION__: JSON.stringify(process.env.npm_package_version ?? '0.0.0'),
  },
})

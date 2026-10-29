import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __filename = fileURLToPath(import.meta.url);
const __dirname = resolve(__filename, "..");

const host = process.env.TAURI_DEV_HOST;
const uiStyles = resolve(__dirname, "../ui", "styles/index.css");

export default defineConfig(async () => ({
  plugins: [vue()],

  resolve: {
    alias: [
      { find: "@ice-paw/ui/styles", replacement: uiStyles },
    ],
  },

  // vendor 分包（0.9.11 生产实案「首启白窗长时间未响应」缓解，2026-10-29）：
  // 版本升级清 WebView2 缓存后冷启动 = JS 重新编译。只拆**首屏必用且稳定**的
  // vue 全家桶 / tauri api 两块（Code Cache 粒度：业务改动不再使其失效 + 并行
  // 编译）。⚠️ 勿加「其余 node_modules 全捕进 vendor-misc」兜底——实测会把
  // highlight.js/vue-cropper 等路由级重依赖聚合出 1.1MB 大杂烩，首屏被迫全量
  // 编译，比不分包更糟；未列依赖留给 rollup 默认按使用方（路由 chunk）分割。
  build: {
    rollupOptions: {
      output: {
        manualChunks(id: string) {
          if (!id.includes("node_modules")) return undefined;
          if (id.includes("node_modules/@tauri-apps")) return "vendor-tauri";
          if (id.includes("node_modules/@vue") || id.includes("node_modules/vue") || id.includes("node_modules/pinia")) return "vendor-vue";
          return undefined;
        },
      },
    },
  },

  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? { protocol: "ws", host, port: 1421 }
      : undefined,
    fs: { allow: [resolve(__dirname, "../ui"), __dirname] },
    watch: { ignored: ["**/src-tauri/**"] },
  },
}));

import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import { resolve } from "path";

// 读取 Tauri dev 模式下的远程 host（用于局域网真机调试）
// types: ["node"] 已在 tsconfig.node.json 中声明
const host = process.env.TAURI_DEV_HOST;

// UI 包样式别名：把 `@ice-paw/ui/styles` 直接指向 UI 源码 styles 入口，
// 加载 tokens + base + index；与预览站 dev/vite.config.ts 保持一致。
// 该别名同时作用于 dev 与 build，确保产物注入完整 design tokens。
const uiRoot = resolve(__dirname, "../ui");
const uiStyles = resolve(uiRoot, "src/styles/index.css");

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [vue()],

  resolve: {
    alias: {
      "@ice-paw/ui/styles": uiStyles,
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    // 允许 dev 模式访问 workspace 内相邻包（避免 fs sandbox 拦截）
    fs: {
      allow: [uiRoot, resolve(__dirname)],
    },
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));

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

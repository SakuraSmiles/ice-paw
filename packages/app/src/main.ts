// 应用入口：创建 Vue 应用实例，挂载 router，然后挂载到 DOM
//
// 样式顺序：先导入 UI 库的 design tokens 与基础样式，再叠加本应用的 global.css
// 全局 CSS 必须在 Vue 实例创建之前引入，确保 :root 上的 token 变量在组件挂载前已注入
import "@ice-paw/ui/styles";
import "./assets/styles/fonts.css"; // 本地字体（@font-face 自托管，离线可用；先于 global 使字体就绪）
import "./assets/styles/global.css";
import "./assets/styles/markdown.css";
import { createApp } from "vue";
import App from "./App.vue";
import router from "./router";
import pinia from "./stores";

const app = createApp(App);
app.use(pinia);
app.use(router);
app.mount("#app");

// 主窗 visible:false（tauri.conf，消灭冷启动「白屏窗口悬着未响应」——窗口迟到
// 但出现即完整界面）。前端挂载完成 = 首帧可渲染，此处主动 show。后端另有 10s
// 兜底 show（前端 JS 挂死时窗口仍会出现，问题可见而非「打不开」）。
import { getCurrentWindow } from "@tauri-apps/api/window";
requestAnimationFrame(() => {
  getCurrentWindow().show();
});

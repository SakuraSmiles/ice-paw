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
//
// 启动加载页退场（2026-09-16）：boot-splash 是 body 级静态 HTML（index.html 内联
// 样式，先于一切 JS 可见、不被 mount 吞掉），亮窗后从这里接管——从亮窗（用户实际
// 能看见的时刻）起算**最少展示 1s**，随后挂 .boot-splash-out 交叉淡出 320ms 并移除
// 节点。JS 挂死时此回调不执行、骨架留存 = 诚实的「启动中」而非空白窗。
import { getCurrentWindow } from "@tauri-apps/api/window";
requestAnimationFrame(() => {
  void getCurrentWindow().show().catch(() => {
    /* 浏览器 dev 无窗口概念——照常按计时退场，防骨架滞留 */
  });
  const splash = document.getElementById("boot-splash");
  window.setTimeout(() => {
    if (!splash) return;
    splash.classList.add("boot-splash-out");
    window.setTimeout(() => splash.remove(), 360);
  }, 1000);
});

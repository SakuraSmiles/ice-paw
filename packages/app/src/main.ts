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
import { reportBootProgress, bootReady } from "./utils/bootProgress";

// 模块执行 = bundle 加载完成的最早信号（此前 splash 只有循环动画占位）
reportBootProgress(15, "加载应用");

const app = createApp(App);
app.use(pinia);
app.use(router);
app.mount("#app");
reportBootProgress(50, "初始化界面");

// 主窗 visible:false（tauri.conf，消灭冷启动「白屏窗口悬着未响应」——窗口迟到
// 但出现即完整界面）。前端挂载完成 = 首帧可渲染，此处主动 show。后端另有 10s
// 兜底 show（前端 JS 挂死时窗口仍会出现，问题可见而非「打不开」）。
//
// 启动加载页退场（2026-09-16 Phase2）：boot-splash 是 body 级静态 HTML（index.html
// 内联样式，先于一切 JS 可见、不被 mount 吞掉），亮窗后从这里接管——退场**双
// 条件**：真实就绪（Sidebar 初始数据完成 → bootProgress 上报 100 → dispatch
// 'icepaw:boot-ready'）且从亮窗起**最少展示 2.5s**（用户拍板 1s 太短），随后挂
// .boot-splash-out 交叉淡出 320ms 并移除节点；~10s 超时无条件退场兜底（上报链
// 断裂/极端慢机不让骨架滞留）。JS 挂死时此回调不执行、骨架留存 = 诚实的
// 「启动中」而非空白窗。
import { getCurrentWindow } from "@tauri-apps/api/window";
requestAnimationFrame(() => {
  void getCurrentWindow().show().catch(() => {
    /* 浏览器 dev 无窗口概念——照常按双条件退场，防骨架滞留 */
  });
  reportBootProgress(60, "加载会话数据");
  const MIN_SPLASH_MS = 2500;
  const BOOT_TIMEOUT_MS = 10000;
  const shownAt = performance.now();
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    const splash = document.getElementById("boot-splash");
    if (!splash) return;
    splash.classList.add("boot-splash-out");
    window.setTimeout(() => splash.remove(), 360);
  };
  // 就绪事件先到 → 等满最少展示时长再退场；先到的是时限 → 直接退（就绪事件
  // 迟到时 finished 已置位，天然幂等）
  const finishWhenAged = () => {
    const elapsed = performance.now() - shownAt;
    if (elapsed >= MIN_SPLASH_MS) finish();
    else window.setTimeout(finishWhenAged, MIN_SPLASH_MS - elapsed);
  };
  window.addEventListener("icepaw:boot-ready", finishWhenAged, { once: true });
  // 竞态补查：Sidebar 初始数据若在监听器挂上前就已完成（bootReady 已置位），
  // 事件已错过——直接进入「等满 2.5s」路径
  if (bootReady()) finishWhenAged();
  window.setTimeout(finish, BOOT_TIMEOUT_MS);
});

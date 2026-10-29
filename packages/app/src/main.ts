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
import { reportBootProgress, bootReady, rosterRemainingMs } from "./utils/bootProgress";
import { getVersion } from "@tauri-apps/api/app";

// 模块执行 = bundle 加载完成的最早信号（此前 splash 只有循环动画占位）
reportBootProgress(15, "加载应用");

// 启动页版本号（右下角档案行上方）：bundle 可执行的第一刻即取——Tauri IPC 此刻已
// 就绪，早于亮窗与一切数据加载，是全链路最早取得点（withGlobalTauri 未开，
// splash 静态期的内联脚本拿不到）。core:default 已含 core:app:default 放行。
void getVersion()
  .then((v) => {
    const verEl = document.getElementById("boot-ver");
    if (verEl) verEl.textContent = `v${v}`;
  })
  .catch(() => {
    /* 浏览器 dev 无 Tauri IPC——留空（.boot-ver:empty 收起占位） */
  });

const app = createApp(App);
app.use(pinia);
app.use(router);
app.mount("#app");
reportBootProgress(50, "初始化界面");

// 主窗 visible:true + backgroundColor=splash 深底（2026-10-27 启动时序第一步）：
// 窗口创建即刻出现（品牌底色），splash 由 webview 并行加载尽早落位。此处 show
// 幂等保留；后端另有 10s 兜底 show（前端 JS 挂死时的保险）。
//
// 启动加载页退场（2026-09-16 Phase2）：boot-splash 是 body 级静态 HTML（index.html
// 内联样式，先于一切 JS 可见、不被 mount 吞掉），亮窗后从这里接管——退场**双
// 条件**：真实就绪（Sidebar 初始数据完成 → bootProgress 上报 100 → dispatch
// 'icepaw:boot-ready'）且从亮窗起**最少展示 2.5s**（用户拍板 1s 太短），随后挂
// .boot-letting 分层让位编舞（~950ms，时间表见 index.html）并移除节点；~10s 超时无条件退场兜底（上报链
// 断裂/极端慢机不让骨架滞留）。JS 挂死时此回调不执行、骨架留存 = 诚实的
// 「启动中」而非空白窗。
import { getCurrentWindow } from "@tauri-apps/api/window";
requestAnimationFrame(() => {
  void getCurrentWindow().show().catch(() => {
    /* 浏览器 dev 无窗口概念——照常按双条件退场，防骨架滞留 */
  });
  reportBootProgress(60, "加载会话数据");
  const MIN_SPLASH_MS = 2500;
  const BOOT_TIMEOUT_MS = 14000; // 名册全序列（三组完整演完）上限 8 行 ~7s + 停顿 0.5s + 欢迎 1.5s + 欢迎后停留 1.5s ≈ 10.5s，兜底须留出余量
  const shownAt = performance.now();
  let finished = false;
  const finish = () => {
    if (finished) return;
    finished = true;
    const splash = document.getElementById("boot-splash");
    if (!splash) return;
    // 分层让位 + 应用入场无感衔接（2026-10-29 演进，取代「先退净再入场」）：
    // 单一状态类 .boot-letting 驱动 splash 各层错峰退场；应用入场信号在编舞**中段**
    // 派发（非节点移除后），让「加载元素退场」与「应用骨架入场」交叠成 crossfade——
    // 透过渐透明的底看见布局逐块生长，消除 ~500ms 纯底色空窗。时间表（ms，
    // 与 index.html 样式区 / AppLayout 入场延迟三处对齐）：
    //   t=0    中央演出收束（400ms）
    //   t=150  容器底色透明（450ms）
    //   t=350  氛围/环场/名册淡出（450ms，至 800 淡净）
    //   t=550  SPLASH_HANDOFF_MS：派发 icepaw:splash-out → app 骨架入场起跑
    //          （AppLayout 侧栏 +150 / 消息区 +300）——此刻中央已收束、环场已
    //          大半淡出，入场落在环场淡净（800）前后，无亮色重叠
    //   t=950  节点移除（编舞全长 + 余量）
    // ⚠️ 勿回退到 t=0 同帧起跑（2026-10-28 实机否决：首页与 0~350ms 满亮度环组
    // 重叠）；勿回退到节点移除后派发（产生底色空窗）。550 是唯一真相源。
    const SPLASH_HANDOFF_MS = 550;
    splash.classList.add("boot-letting");
    window.setTimeout(
      () => window.dispatchEvent(new CustomEvent("icepaw:splash-out")),
      SPLASH_HANDOFF_MS,
    );
    window.setTimeout(() => splash.remove(), 950); // 编舞全长 + 余量（三处对齐）
  };
  // 就绪事件先到 → 等满「最少展示时长 + 名册演完」再退场；先到的是时限 → 直接
  // 退（就绪事件迟到时 finished 已置位，天然幂等）。名册三组全序列（agents→
  // projects→sessions 一组完整演完下一组才登场）约 9s，退场须等名册收尾，否则
  // 演到一半就被切走。
  const finishWhenAged = () => {
    const minWait = MIN_SPLASH_MS - (performance.now() - shownAt);
    const wait = Math.max(minWait, rosterRemainingMs());
    if (wait <= 0) finish();
    else window.setTimeout(finishWhenAged, wait);
  };
  window.addEventListener("icepaw:boot-ready", finishWhenAged, { once: true });
  // 竞态补查：Sidebar 初始数据若在监听器挂上前就已完成（bootReady 已置位），
  // 事件已错过——直接进入「等满 2.5s」路径
  if (bootReady()) finishWhenAged();
  window.setTimeout(finish, BOOT_TIMEOUT_MS);
});

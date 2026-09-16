// utils/bootProgress.ts — 启动加载页真实进度上报（2026-09-16 Phase2 批：用户
// 反馈「进度条像循环动画」——初版 indeterminate 滑动条只是占位动效，不代表任何
// 真实状态）。进度来自启动链路的真实节点：
//   main.ts 模块执行 15「加载应用」→ mount 50「初始化界面」→ 亮窗 60「加载会话
//   数据」→ Sidebar 初始数据完成 100「就绪」。
// 退场条件 = 100% 且亮窗起最少展示 2.5s（main.ts 监听 icepaw:boot-ready 双条件
// 汇合，~10s 超时兜底防上报链断裂骨架滞留）。
//
// 设计约束：
// - **单调递增钳制**：上报点乱序到达时倒退会让进度条回跳 = 又一种假进度；
// - **null-safe**：splash 退场后 / jsdom 组件测试环境无 #boot-splash——全部
//   no-op（Sidebar 等组件测试可正常 mount，勿在调用侧加存在性判断）；
// - **首次上报切换形态**：indeterminate 循环动画 → 确定性 width（挂
//   .boot-determinate：animation:none + transition，见 index.html 内联样式）。

let current = 0;
let readyDispatched = false;

/** 真实进度是否已到 100（供 main.ts 补查「监听器挂上前就已就绪」的竞态）。 */
export function bootReady(): boolean {
  return readyDispatched;
}

/**
 * 上报启动进度（0-100，钳制单调递增）。label 同时写入 splash 状态行文案。
 */
export function reportBootProgress(pct: number, label?: string): void {
  const splash = document.getElementById("boot-splash");
  if (!splash) return; // 退场后/测试环境：no-op
  const next = Math.min(100, Math.max(current, pct));
  current = next;
  const fill = splash.querySelector<HTMLElement>(".boot-bar-fill");
  if (fill) {
    fill.classList.add("boot-determinate"); // 循环动画 → 确定性 width（幂等）
    fill.style.width = `${next}%`;
  }
  if (label) {
    const status = splash.querySelector<HTMLElement>(".boot-status");
    if (status) status.textContent = label;
  }
  if (current >= 100 && !readyDispatched) {
    readyDispatched = true;
    window.dispatchEvent(new CustomEvent("icepaw:boot-ready"));
  }
}

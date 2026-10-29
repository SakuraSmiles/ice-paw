// utils/bootProgress.ts — 启动加载页真实进度上报（2026-09-16 Phase2 批：用户
// 反馈「进度条像循环动画」——初版 indeterminate 滑动条只是占位动效，不代表任何
// 真实状态）。进度来自启动链路的真实节点：
//   main.ts 模块执行 15「加载应用」→ mount 50「初始化界面」→ 亮窗 60「加载会话
//   数据」→ Sidebar 初始数据完成 100「就绪」。
// 退场条件 = 100% 且亮窗起最少展示 2.5s（main.ts 监听 icepaw:boot-ready 双条件
// 汇合，main.ts BOOT_TIMEOUT_MS=14s 超时兜底防上报链断裂骨架滞留）。
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

// ---- 工业制图语言（2026-10-27 DSH 同源升格，档 B）----
// 阶段英文小字映射（boot-sub 行）：按上报节点百分比推导，与中文 label 同源
// （15 加载应用 / 50 初始化界面 / 60 加载会话数据 / 100 就绪）。
const STAGE_EN: ReadonlyArray<readonly [number, string]> = [
  [100, "READY"],
  [60, "LOADING SESSION DATA"],
  [50, "INITIALIZING INTERFACE"],
  [15, "LOADING APPLICATION"],
];
function stageEnOf(pct: number): string {
  for (const [p, s] of STAGE_EN) if (pct >= p) return s;
  return "BOOT SEQUENCE";
}

// 乱码→落定（roll）：DSH identity-glyphs decode 同款视觉——左→右逐位锁定，
// 未锁定位显示随机工业字符。只写视觉层（.boot-status-visual，aria-hidden），
// 权威层 .boot-status 恒同步直写（测试契约 + 屏幕阅读器）。
const SCRAMBLE_GLYPHS = "!<>-_\\/[]{}=+*^?#";
let rollTimer = 0;
let lastVisualLabel = "";
function rollVisualLabel(el: HTMLElement, text: string): void {
  window.clearInterval(rollTimer);
  lastVisualLabel = text;
  const reduced =
    typeof window.matchMedia === "function" &&
    window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  if (reduced || !text) {
    el.textContent = text;
    return;
  }
  const chars = [...text];
  const start = performance.now();
  const dur = Math.min(380, 140 + chars.length * 22);
  rollTimer = window.setInterval(() => {
    const t = (performance.now() - start) / dur;
    const locked = Math.floor(Math.max(0, Math.min(1, t)) * chars.length);
    el.textContent = chars
      .map((c, i) =>
        i < locked || c === " "
          ? c
          : SCRAMBLE_GLYPHS[Math.floor(Math.random() * SCRAMBLE_GLYPHS.length)],
      )
      .join("");
    if (locked >= chars.length) {
      window.clearInterval(rollTimer);
      el.textContent = text; // 终态精确落定（随机流不留残影）
    }
  }, 30);
}

/**
 * 就绪名册注入（DSH dsh-lettering 遮罩划出揭示）：按组把已到达的数据逐行
 * append（行按到达序呈现，数据一次性到齐）。三组（agents/projects/sessions）
 * 各自独立状态行 + 列表：加载态九宫格 → 结果态环形 check，文字遮罩划出换结果。
 * **顺序编排**：一组完整演完（含换字遮罩解除）下一组才登场，非三组重叠；组演
 * 完移除行占位（标题保留为常驻标签），下一组紧跟不留空白。
 * splash 已退场/组容器不存在时静默 no-op。
 */
const ROSTER_DONE_TEXT: Readonly<Record<string, (n: number) => string>> = {
  agents: (n) => `${n} 名智能体已上线`,
  projects: (n) => `${n} 项目解析完成`,
  sessions: () => "会话数据恢复完成",
};

// 组间顺序编排（2026-10-27）：三组（agents→projects→sessions）演出时间轴**串行
// 全完成**——下一组在上一组「状态行换结果 + 换字遮罩解除（演出彻底完成）」之后
// 才起跑，实现「一组完整演完，下一组才登场」而非重叠。节奏已压缩（停留 900→250、
// 退场余量收紧、组间停顿 150→120），每组内容上限 8 行，整场约 4~7s。演完 → 停顿
// ROSTER_BEAT_MS → 中心「欢迎使用」同槽换字（打字机 ~0.5s）→ 欢迎后停留 1.2s 再退场
// （rosterRemainingMs 全计入）。组演完统一移除行占位，下一
// 组紧跟（标题 AGENT/PROJECTS/SESSIONS 保留为常驻标签）。agents 恒为序列头（Sidebar
// 固定调用序），在此重置舞台时钟，保证每次启动/每个测试都从 0 起跑、时序确定。
const GROUP_GAP = 120; // 上一组演完后到下一组入场的停顿（一 beat）
const ROSTER_BEAT_MS = 500; // 名册演完到「欢迎使用」起演的停顿（名册先、欢迎后的清晰分隔）
const CENTER_SETTLE_MS = 500; // 「欢迎使用/WELCOME」同槽换字打字机演出预算（淡入 200 + 打字 320ms，2026-10-28 提速与其他入场同速带）
const WELCOME_HOLD_MS = 1200; // 欢迎展示完后到退场的停留（用户拍板 1.2s，随后随整体淡出）
let rosterClock = 0;
let rosterEndAt = 0;    // 名册最后一组演出彻底结束的绝对时刻（performance.now() 域）
let settleTimer = 0;    // 中心收尾定时器（clear+reschedule，最终落在最后一组真正演完时刻）
let rosterDoneCb: (() => void) | null = null; // 名册演完回调（Sidebar 借此推迟「就绪」100% 上报）
let rosterDoneTimer = 0; // 名册演完回调定时器（clear+reschedule，与 settleTimer 同落在最后一组演完时刻）

/** 注册「名册全部演完」回调——Sidebar 用它把「就绪」100% 上报推迟到名册最后一组
 *  真正演完（而非三组数据刚注入时），避免中心「就绪/欢迎」在名册仍演出时提前出现
 *  （总/子先后关系颠倒）。单槽赋值（重复注册只保留最后一次，Sidebar 挂载即一次）。 */
export function onRosterDone(cb: () => void): void {
  rosterDoneCb = cb;
}

export function rosterAppend(groupId: string, labels: readonly string[]): void {
  const splash = document.getElementById("boot-splash");
  const host = splash?.querySelector<HTMLElement>(`#roster-${groupId}`);
  if (!splash || !host) return;
  if (groupId === "agents") {
    rosterClock = 0; // 序列头重置
    rosterEndAt = 0;
  }

  const start = rosterClock; // 本组起点 = 前面所有组累计偏移
  const rows: HTMLElement[] = [];
  for (const label of labels) {
    const row = document.createElement("div");
    row.className = "roster-row";
    const span = document.createElement("span");
    span.textContent = label;
    const bar = document.createElement("i");
    row.append(span, bar);
    host.appendChild(row);
    rows.push(row);
  }
  // 三阶段编排（与预览台同参数）：入场 80ms/行 → 揭示 +200ms → 停留 250ms →
  // 退场覆盖 60ms/行 → 隐藏后组演完统一移除占位（下一组紧跟、不留空白）。
  const STEP = 80, IN_TO_REVEAL = 200, HOLD = 250, EXIT_STEP = 60;
  const allDone = rows.length * STEP + 250;
  const statusAt = start + allDone + HOLD + rows.length * EXIT_STEP + 300;
  // 本组演出彻底结束（编舞节拍）= statusAt + 320；下一组从此起跑（+ 停顿）。
  // 记录绝对结束时刻供启动页「等名册演完再退场」使用。
  // ⚠️ 用「本次调用的 performance.now() + 相对 groupEnd」而非「序列头基准 + 累计
  // groupEnd」：三组 rosterAppend 之间夹着 Sidebar 的 await（project.load /
  // chat.loadConversations 真实耗时），序列头基准会把这些真实等待漏算，导致名册
  // 演完时刻被低估 → 中心收尾提前、总/子先后关系颠倒（production 实案）。
  // ⚠️ 320 是「下一组起跑节拍」（时序测试锁定的契约值），**不等于**换字遮罩的
  // CSS 全程——reveal 余韵（SWAP_TOTAL_MS=550）允许与下一组起跑重叠（不同
  // 元素非阻塞余韵），摘 .swapping 类的 timer 用 550（批次 Z/B3：原 320 摘类
  // 早于 reveal 播完，遮罩从半盖态瞬跳全开）。
  const SWAP_TOTAL_MS = 550;
  const groupEnd = statusAt + 320;
  rosterClock = groupEnd + GROUP_GAP;
  rosterEndAt = Math.max(rosterEndAt, performance.now() + groupEnd);

  // 中心收尾：名册最后一组演完 + 停顿（ROSTER_BEAT_MS）后触发「欢迎使用」同槽换字（打字机 + 副题 WELCOME）
  // （clear+reschedule 用单调递增的 rosterEndAt 重排，最终定时器落在最后一组真正演完的
  // 绝对时刻，而非每组各响一次）。
  window.clearTimeout(settleTimer);
  const center = splash.querySelector<HTMLElement>(".boot-center");
  if (center) {
    settleTimer = window.setTimeout(
      () => center.classList.add("welcoming"),
      Math.max(0, rosterEndAt + ROSTER_BEAT_MS - performance.now()),
    );
  }
  // 名册演完回调：与 settleTimer 同源重排，落在最后一组真正演完时刻（无停顿——「就绪」
  // 是名册演完的第一反应，欢迎语再隔一拍登场）。
  window.clearTimeout(rosterDoneTimer);
  if (rosterDoneCb) {
    rosterDoneTimer = window.setTimeout(
      () => rosterDoneCb?.(),
      Math.max(0, rosterEndAt - performance.now()),
    );
  }

  // 组标题 + 状态行也按组起点顺序点亮（不是三组同时点亮）
  setTimeout(() => host.classList.add("title-on"), start);
  const statusEl = host.querySelector<HTMLElement>(".roster-status");
  if (statusEl) {
    setTimeout(() => statusEl.classList.add("on"), start);
    setTimeout(() => {
      statusEl.classList.add("done"); // 持久：九宫格→done 图标（切换后不退）
      statusEl.classList.add("swapping"); // 临时：文字遮罩动画（cover 200ms 盖旧文字）
      const textEl = statusEl.querySelector<HTMLElement>(".status-text");
      // 遮罩 cover 200ms 盖满旧文字后才换新文字（与 CSS roster-status-cover 时长对齐），
      // 之后 reveal 揭开露出结果文案——避免新文字在遮罩覆盖前闪现。
      setTimeout(() => {
        if (textEl) textEl.textContent = ROSTER_DONE_TEXT[groupId](labels.length);
      }, 200);
      setTimeout(() => statusEl.classList.remove("swapping"), SWAP_TOTAL_MS);
    }, statusAt);
  }

  rows.forEach((row, i) => {
    setTimeout(() => row.classList.add("in"), start + i * STEP);
    setTimeout(() => row.classList.add("reveal"), start + i * STEP + IN_TO_REVEAL);
  });
  rows.forEach((row, i) => {
    const exitAt = start + allDone + HOLD + i * EXIT_STEP;
    setTimeout(() => row.classList.add("exit"), exitAt);
    setTimeout(() => row.classList.add("gone"), exitAt + 300);
  });

  // 组演完：一次性移除行占位（display:none）——下一组紧跟、不留第一组全量空白。
  // 全完成顺序下下一组此刻尚未点亮（opacity 0），移除占位不可见、不顶行。
  setTimeout(() => host.classList.add("collapsed"), statusAt);
}

/** 名册演完 + 停顿 +「欢迎使用」揭示 + 欢迎后停留，离可退场还剩多少毫秒（启动页据此
 *  等全部子动画演绎完成再退场——含 ~1.5s 欢迎收尾 + ~1.5s 欢迎后停留，保证用户看清欢迎语
 *  并有余裕观看名册信息再进入首屏）。 */
export function rosterRemainingMs(): number {
  return Math.max(
    0,
    rosterEndAt + ROSTER_BEAT_MS + CENTER_SETTLE_MS + WELCOME_HOLD_MS - performance.now(),
  );
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
    // 权威层：同步直写（既有测试契约锁定此行为，勿改成异步）
    const status = splash.querySelector<HTMLElement>(".boot-status");
    if (status) status.textContent = label;
    // 视觉层：乱码落定（阶段推进的动效读数）；label 未变不重播
    const visual = splash.querySelector<HTMLElement>(".boot-status-visual");
    if (visual && label !== lastVisualLabel) rollVisualLabel(visual, label);
  }
  // 英文阶段小字（boot-sub 行）
  const sub = splash.querySelector<HTMLElement>(".boot-sub");
  if (sub) sub.textContent = stageEnOf(next);
  // 生长动画驱动（DSH engraving/locking）：--boot-prog 0~1 → 环场弧长/刻度揭示
  if (splash) {
    splash.style.setProperty("--boot-prog", String(next / 100));
    const rings = splash.querySelector(".boot-rings");
    if (rings && next >= 100 && !rings.classList.contains("ready")) {
      rings.classList.add("ready"); // 就绪脉冲一次
    }
  }
  if (current >= 100 && !readyDispatched) {
    readyDispatched = true;
    window.dispatchEvent(new CustomEvent("icepaw:boot-ready"));
  }
}

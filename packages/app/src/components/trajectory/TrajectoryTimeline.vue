<!--
  TrajectoryTimeline — 顶部瀑布图时间轴（86px = 顶部留白 6 + 泳道 64 + 刻度轴 16，仿 dsh TrajectoryTimeline）

  dsh 的核心洞见：x 轴是「投影」不是「时间」——纯墙钟会被会话空闲压垮（dsh 自己把
  actual 模式藏了；我们秒级时间戳 + 跨小时会话更撑不起）。两种投影（工具栏「耗时」开关）：
  - sequence（默认）：x = 事件槽位，每事件等宽一格。不依赖时间戳、无空白、均匀铺开。
  - duration：真实耗时区间 + 空闲压缩（dsh sweep 算法：按 start 排序，维护
    coveredUntil = 之前所有 span 的 max end，空档累计删除、后续整体左移；并发区间保留）。
    条宽两级来源：tool_execution / assistant_message 的 duration_ms 优先；无记录事件回退
    「距本 turn 上一事件间隔」（隐式耗时——事件完成时落库，间隔≈真实工作窗口，如
    assistant 生成时间；turn 首事件不回退，轮间空闲留给压缩删）。
    底轴此模式标相对时间（+12s），墙钟进 hover tooltip。

  4 泳道 + 左侧 56px 标签列（学 dsh：靠标签分区，泳道间不画分隔线；空泳道标签淡化恒显，
  道位稳定；泳道名用英文短名，与 canvas 内英文刻度同语言；顶部 6px 留白，标签列与
  canvas 泳道共用 TOP_PAD/LANE_H 保持逐像素对齐）：
    0 User（蓝）  user_message / attachment_stored —— 输入侧（turn_context 折进表格轮次头，不上图）
    1 Model（灰） assistant_message / summary_* / modal_adapted / message_error / message_discarded
    2 Tools（琥珀）tool_execution（tool_result_message 是 DB 结果行镜像，不上图）
    3 Hooks（绿） hook_injected —— 外挂逻辑干预对话流的审计面
  turn_ended 不占泳道：画成贯穿竖线的 turn 边界层（dsh turnBoundaries 同构），
  序号模式底轴在边界处标「第 N 轮」（编号与表格 buildRows 对齐：孤儿桶占号但不标）。

  交互：hover 提示 · 点击块跳转表格行（emit pick(seq)）· 滚轮缩放（光标锚定）· 拖拽平移。
  canvas 绘制（DPR 感知），数千事件只画一次矩形批，无 DOM 压力。
-->
<script setup lang="ts">
import { computed, onMounted, onBeforeUnmount, ref, watch } from "vue";
import type { SessionEvent } from "../../types";

const props = defineProps<{
  events: SessionEvent[];
  /** 当前选中行的 seq（对应 span 高亮环） */
  selectedSeq: number | null;
  /** 投影模式：sequence = 序号等宽（默认）；duration = 真实耗时 + 空闲压缩 */
  mode: "sequence" | "duration";
  /** 窗口前（更早分页一侧）的全局轮次数（M3：底轴「第 N 轮」与表格同源偏移） */
  turnOffset: number;
  /** 尾部优先分页下还有更早的事件（左缘显示「…」加载入口） */
  hasEarlier: boolean;
  loadingEarlier: boolean;
}>();
const emit = defineEmits<{
  pick: [seq: number];
  "load-earlier": [];
}>();

/** 顶部留白（泳道与标签不贴上缘，canvas 与标签列共用同一偏移保对齐） */
const TOP_PAD = 6;
const HOST_H = 86; // TOP_PAD(6) + 泳道区(64) + 刻度轴(16)
const LANE_H = 16;
const LANES_N = 4;
const LANES_H = LANE_H * LANES_N; // 64（泳道区高度，之下是刻度轴）
const LANE_LABELS = ["User", "Model", "Tools", "Hooks"] as const;

interface Span {
  seq: number;
  lane: number;
  /** 序号模式 run 合并用：kind 语义键（lane 内再按 kind 细分，模型道混 summary/error 不混并） */
  kindKey: string;
  start: number; // 域单位：sequence=槽位；duration=压缩后 ms
  end: number;
  isError: boolean;
  /** tooltip 文案（kind 中文 + 工具名等补充） */
  label: string;
  /** 墙钟 HH:MM:SS（时间信息不进布局维度，进 tooltip） */
  timeText: string;
  /** 工具/assistant 真实耗时记录（tooltip 显示；duration 模式即条宽；null = 隐式兜底） */
  durMs: number | null;
}

interface TurnBoundary {
  /** 与表格 turnIndex 对齐（1-based 展示「第 N 轮」） */
  turnIndex: number;
  /** 锚在 turn 首个可上图事件上（duration 模式空闲压缩后位置随之更新，勿按值拷贝） */
  span: Span;
}

const track = ref<HTMLDivElement | null>(null);
const canvas = ref<HTMLCanvasElement | null>(null);
const tooltip = ref<{ x: number; y: number; text: string } | null>(null);
const empty = ref(false);
const laneActive = ref<boolean[]>([false, false, false, false]);
/** 视口贴住数据域起点时显示「…」加载更早入口（draw 内刷新） */
const earlierVisible = ref(false);

let spans: Span[] = [];
let boundaries: TurnBoundary[] = [];
let d0 = 0; // 数据域
let d1 = 1;
let t0 = 0; // 视口（缩放/平移修改）
let t1 = 1;
let dpr = 1;
let width = 0;
let ro: ResizeObserver | null = null;
let redrawRaf = 0;

function laneOf(kind: SessionEvent["kind"]): number {
  switch (kind) {
    case "user_message":
    case "turn_context":
    case "attachment_stored":
      return 0;
    case "assistant_message":
    case "summary_created":
    case "summary_updated":
    case "modal_adapted":
    case "message_error":
    case "message_discarded":
      return 1;
    case "tool_execution":
      return 2;
    case "hook_injected":
      return 3;
    default:
      return 1; // 未知未来 kind 兜底模型道（中性灰）
  }
}

const KIND_LABELS: Partial<Record<SessionEvent["kind"], string>> = {
  user_message: "用户消息",
  turn_context: "轮上下文",
  attachment_stored: "附件落库",
  assistant_message: "模型回复",
  summary_created: "摘要生成",
  summary_updated: "摘要更新",
  modal_adapted: "视觉适配",
  message_error: "错误",
  message_discarded: "消息丢弃",
  tool_execution: "工具",
  hook_injected: "钩子注入",
};

function fmtClock(t: number): string {
  const d = new Date(t);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}

function fmtRel(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  const s = ms / 1000;
  if (s < 60) return `${s.toFixed(1)}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${Math.round(s % 60)}s`;
  return `${Math.floor(m / 60)}h${m % 60}m`;
}

/**
 * 事件流 → 域内 span + turn 边界。
 * 轮次编号镜像 buildRows：按 turn-key 切换递增、孤儿桶（turn_id=null）占号但不画边界。
 * turn_context / turn_ended / tool_result_message 不生成 span（零耗时元数据 / 边界层 / DB 镜像）。
 */
function buildSpans() {
  spans = [];
  boundaries = [];
  laneActive.value = [false, false, false, false];

  const seqMode = props.mode === "sequence";
  let slot = 0;
  let curTk: string | null = null;
  // M3：窗口首桶的全局轮号从偏移起算（0 偏移时 -1+1=0，行为与原实现一致）
  let turnIndex = props.turnOffset - 1;
  /** 本 turn 尚未打边界点（遇到首个可上图事件时落点）；null = 孤儿桶/已打点 */
  let pendingTurn: number | null = null;
  /**
   * 隐式耗时兜底（duration 模式）：上一可上图事件的墙钟（同 turn 内）。
   * 无真实耗时的事件（assistant 纪元早期无 duration_ms / 工具瞬时）条形回退为
   * [t_prev, t]——事件在完成时落库，该间隔几乎就是真实工作窗口（如 assistant 的
   * 生成时间）。turn 首事件不回退：轮间空闲不属于任何事件，留给压缩算法删。
   * 工具/assistant 的真实 duration_ms 永远优先，前端不覆盖。
   */
  let prevT: number | null = null;

  for (const ev of props.events) {
    const tk = ev.turn_id ?? "__orphan__";
    if (tk !== curTk) {
      turnIndex += 1;
      curTk = tk;
      pendingTurn = ev.turn_id != null ? turnIndex : null;
      prevT = null; // 跨 turn：不给新 turn 的首事件挂上一轮的尾巴
    }
    // turn_context / turn_ended / tool_result_message 不上图：配置快照是零耗时
    // 元数据（折进表格轮次头），画出来只会让「块数 ≠ 表格可见行数」（用户道每轮
    // 恒 2 块 = turn_context + user_message，但表里只有 1 行）；后两者同理（边界层/镜像）。
    if (ev.kind === "turn_ended" || ev.kind === "turn_context" || ev.kind === "tool_result_message") continue;

    const lane = laneOf(ev.kind);
    const t = Date.parse(ev.created_at);
    let dur: number | null = null; // null = 无真实耗时记录
    let isError = false;
    let label = KIND_LABELS[ev.kind] ?? ev.kind;
    if (ev.kind === "tool_execution") {
      const p = ev.payload as { duration_ms?: number; is_error?: boolean; tool_name?: string };
      dur = Math.max(0, p.duration_ms ?? 0);
      isError = !!p.is_error;
      if (p.tool_name) label = `工具 ${p.tool_name}`;
    } else if (ev.kind === "assistant_message") {
      const p = ev.payload as { duration_ms?: number };
      if (typeof p.duration_ms === "number" && p.duration_ms >= 0) dur = p.duration_ms;
    } else if (ev.kind === "message_error") {
      isError = true;
    }
    // 隐式耗时兜底：无真实耗时 → 距本 turn 上一事件的间隔（首事件 prevT=null → 0）
    const effDur = dur ?? (prevT !== null && Number.isFinite(t) ? Math.max(0, t - prevT) : 0);

    const timeText = Number.isFinite(t) ? fmtClock(t) : "--:--:--";
    const span: Span = seqMode
      ? { seq: ev.seq, lane, kindKey: ev.kind, start: slot, end: slot + 1, isError, label, timeText, durMs: dur }
      : { seq: ev.seq, lane, kindKey: ev.kind, start: t - effDur, end: t, isError, label, timeText, durMs: dur };
    if (!Number.isFinite(span.start)) continue; // duration 模式下时间不可解析 → 不上图
    spans.push(span);
    laneActive.value[lane] = true;
    if (pendingTurn !== null) {
      boundaries.push({ turnIndex: pendingTurn, span });
      pendingTurn = null;
    }
    if (Number.isFinite(t)) prevT = t;
    slot += 1;
  }

  if (!spans.length) {
    empty.value = props.events.length > 0; // 有事件但全部不可投影（罕见）
    d0 = 0;
    d1 = 1;
    t0 = 0;
    t1 = 1;
    return;
  }
  empty.value = false;

  if (!seqMode) {
    // 空闲压缩（dsh sweep）：按 start 排序，coveredUntil 之前的 max end；
    // gap = span.start - coveredUntil > 0 时累计删除，后续整体左移；并发区间保留。
    const sorted = [...spans].sort((a, b) => a.start - b.start || a.end - b.end);
    const offset = new Map<Span, number>();
    let removed = 0;
    let covered: number | null = null;
    for (const s of sorted) {
      if (covered !== null && s.start > covered) removed += s.start - covered;
      offset.set(s, removed);
      covered = covered === null ? s.end : Math.max(covered, s.end);
    }
    for (const s of spans) {
      const o = offset.get(s) ?? 0;
      s.start -= o;
      s.end -= o;
    }
    let min = Infinity;
    let max = -Infinity;
    for (const s of spans) {
      if (s.start < min) min = s.start;
      if (s.end > max) max = s.end;
    }
    const pad = Math.max((max - min) * 0.02, 200);
    d0 = min - pad;
    d1 = max + pad;
  } else {
    d0 = 0;
    d1 = spans.length;
  }
  t0 = d0;
  t1 = d1;
}

/** CSS 变量取值（canvas fillStyle 不解析 var()，需先问 computed style） */
function cssVar(name: string, fallback: string): string {
  const v = getComputedStyle(track.value ?? document.documentElement).getPropertyValue(name).trim();
  return v || fallback;
}

let colors: Record<string, string> = {};
function resolveColors() {
  colors = {
    danger: cssVar("--ip-danger-base", "#B83D3D"),
    lane0: cssVar("--ip-primary-500", "#4680C2"),
    lane1: cssVar("--ip-gray-500", "#6B7785"),
    lane2: cssVar("--ip-warning-base", "#B8862A"),
    lane3: cssVar("--ip-success-base", "#2D8B66"),
    border: cssVar("--ip-color-border-default", "#CFD5DD"),
    tertiary: cssVar("--ip-color-text-tertiary", "#6B7785"),
    mono: cssVar("--ip-font-mono", "monospace"),
  };
}

function draw() {
  const cvs = canvas.value;
  const el = track.value;
  if (!cvs || !el) return;
  width = el.clientWidth;
  if (width <= 0) return;
  dpr = window.devicePixelRatio || 1;
  cvs.width = Math.round(width * dpr);
  cvs.height = Math.round(HOST_H * dpr);
  cvs.style.width = `${width}px`;
  cvs.style.height = `${HOST_H}px`;
  const ctx = cvs.getContext("2d");
  if (!ctx) return;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, width, HOST_H);
  resolveColors();
  earlierVisible.value = props.hasEarlier && t0 <= d0 + 1e-6;

  const domain = Math.max(1e-6, t1 - t0);
  const x = (t: number) => ((t - t0) / domain) * width;

  // ---- turn 边界层：贯穿泳道区的竖线（画在条形之下；数据域起点处的首边界不画，dsh 同款）----
  ctx.strokeStyle = colors.border;
  ctx.globalAlpha = 0.6;
  ctx.lineWidth = 1;
  for (const b of boundaries) {
    const fx = x(b.span.start);
    if (fx <= 0 || fx > width) continue;
    const xx = Math.round(fx) + 0.5;
    ctx.beginPath();
    ctx.moveTo(xx, TOP_PAD);
    ctx.lineTo(xx, TOP_PAD + LANES_H);
    ctx.stroke();
  }
  ctx.globalAlpha = 1;

  /** 底部刻度轴（事件条两路径——稀疏/列聚合——末尾统一走；先于事件条定义避免 TDZ） */
  const finishAxis = () => {
    ctx.fillStyle = colors.tertiary;
    ctx.textAlign = "left";
    ctx.textBaseline = "bottom";
    ctx.font = `9px ${colors.mono}`;
    if (props.mode === "sequence") {
      // 轮次号刻度：在 turn 边界处标「第 N 轮」，过密时只画刻度不画字
      let lastTextEnd = -Infinity;
      const axisY = TOP_PAD + LANES_H;
      for (const b of boundaries) {
        const xx = x(b.span.start);
        if (xx <= 0 || xx > width) continue;
        ctx.fillRect(Math.round(xx) + 0.5, axisY, 1, 3);
        if (xx > lastTextEnd + 8 && xx < width - 36) {
          ctx.fillText(`第 ${b.turnIndex + 1} 轮`, xx + 3, HOST_H - 3);
          lastTextEnd = xx + 3 + (b.turnIndex >= 9 ? 42 : 36);
        }
      }
    } else {
      // 相对时间刻度（以数据域起点为 0，平移稳定；墙钟在 tooltip）
      const ticks = 6;
      const axisY = TOP_PAD + LANES_H;
      for (let i = 0; i <= ticks; i++) {
        const tt = t0 + (domain * i) / ticks;
        const xx = x(tt);
        ctx.fillRect(Math.round(xx) + 0.5, axisY, 1, 3);
        if (i < ticks) ctx.fillText(`+${fmtRel(Math.max(0, tt - d0))}`, xx + 3, HOST_H - 3);
      }
    }
  };

  // ---- 事件条 ----
  // 待绘条目（统一两种模式的产出形态）：start/end 为域单位，selected 含 run 语义。
  interface DrawItem { start: number; end: number; lane: number; isError: boolean; hasSelected: boolean; }
  const items: DrawItem[] = [];
  if (props.mode === "sequence") {
    // run 合并：连续同 (lane, kind) 画成一条矩形（seq 序 = slot 序，天然相邻）。
    // 密集时段（如连续工具调用/多轮 assistant）不再梳齿化；命中 run 的 tooltip
    // 显示数量与总耗时，click 定位 run 末条（最接近「这组刚结束」）。
    let i = 0;
    while (i < spans.length) {
      const first = spans[i];
      let j = i;
      while (
        j + 1 < spans.length &&
        spans[j + 1].lane === first.lane &&
        spans[j + 1].kindKey === first.kindKey &&
        spans[j + 1].isError === first.isError
      ) j++;
      const run = spans.slice(i, j + 1);
      items.push({
        start: first.start,
        end: run[run.length - 1].end,
        lane: first.lane,
        isError: first.isError,
        hasSelected: run.some((r) => r.seq === props.selectedSeq),
      });
      i = j + 1;
    }
  } else {
    for (const s of spans) {
      items.push({ start: s.start, end: s.end, lane: s.lane, isError: s.isError, hasSelected: s.seq === props.selectedSeq });
    }
  }

  // 密度自适应（M1）：平均每像素 >2 条时条宽必然亚像素，min-width 强制 + gap 逻辑
  // 会把数千个 ≥2px 矩形叠成糊块——切换像素列聚合（每列每道至多 1 个矩形，
  // 连续同值段合并后绘制调用数 ≤ width×lanes），zoom-out 千轮依然锐利。
  if (items.length > width * 2) {
    // bit0=覆盖 bit1=错误 bit2=选中
    const cov = [0, 1, 2, 3].map(() => new Uint8Array(width));
    for (const it of items) {
      const c0 = Math.max(0, Math.floor(x(it.start)));
      const c1 = Math.min(width - 1, Math.ceil(x(it.end)));
      for (let c = c0; c <= c1; c++) {
        cov[it.lane][c] |= 1 | (it.isError ? 2 : 0) | (it.hasSelected ? 4 : 0);
      }
    }
    for (let lane = 0; lane < LANES_N; lane++) {
      const arr = cov[lane];
      const y = TOP_PAD + lane * LANE_H + 2;
      const h = LANE_H - 5;
      let c = 0;
      while (c < width) {
        if (!(arr[c] & 1)) { c++; continue; }
        const err = !!(arr[c] & 2);
        const sel = !!(arr[c] & 4);
        let e = c;
        while (e + 1 < width && (arr[e + 1] & 1) && !!(arr[e + 1] & 2) === err && !!(arr[e + 1] & 4) === sel) e++;
        ctx.fillStyle = err ? colors.danger : colors[`lane${lane}`];
        if (sel) {
          ctx.globalAlpha = 1;
          ctx.fillRect(c - 1, y - 1.5, e - c + 3, h + 3); // 选中高亮环（加宽描边感）
          ctx.globalAlpha = 0.85;
        } else {
          ctx.globalAlpha = 0.8;
        }
        ctx.fillRect(c, y, e - c + 1, h);
        c = e + 1;
      }
    }
    ctx.globalAlpha = 1;
    return finishAxis();
  }

  /** 稀疏路径：单条/run 绘制 */
  const drawItem = (it: DrawItem) => {
    const y = TOP_PAD + it.lane * LANE_H + 2;
    const h = LANE_H - 5;
    let x0 = x(it.start);
    let w = Math.max(2, x(it.end) - x0);
    if (x0 + w < 0 || x0 > width) return; // 视口裁剪
    // 相邻条间细缝（dsh 的 8%·≤1px gap），密堆时仍可分辨
    if (w > 6) {
      const g = Math.min(w * 0.08, 1);
      x0 += g;
      w -= 2 * g;
    }
    ctx.fillStyle = it.isError ? colors.danger : colors[`lane${it.lane}`];
    if (it.hasSelected) {
      ctx.globalAlpha = 1;
      ctx.fillRect(x0 - 1, y - 1.5, w + 2, h + 3); // 选中高亮环（加宽描边感）
      ctx.globalAlpha = 0.85;
    } else {
      ctx.globalAlpha = 0.8;
    }
    ctx.fillRect(x0, y, w, h);
  };
  for (const it of items) drawItem(it);
  ctx.globalAlpha = 1;
  finishAxis();
}

function scheduleRedraw() {
  if (redrawRaf) return;
  redrawRaf = requestAnimationFrame(() => {
    redrawRaf = 0;
    draw();
  });
}

// ---- 交互 ----

/** run 合并索引（sequence 模式 hover/click 用）：run 首位 span + 末位 seq + 数量 */
interface RunHit {
  first: Span;
  endAt: number;
  count: number;
  totalMs: number | null;
}
let runHits: RunHit[] = [];

function rebuildRunHits() {
  runHits = [];
  if (props.mode !== "sequence") return;
  let i = 0;
  while (i < spans.length) {
    const first = spans[i];
    let j = i;
    while (
      j + 1 < spans.length &&
      spans[j + 1].lane === first.lane &&
      spans[j + 1].kindKey === first.kindKey &&
      spans[j + 1].isError === first.isError
    ) j++;
    const run = spans.slice(i, j + 1);
    const totalMs = run.every((r) => r.durMs != null)
      ? run.reduce((n, r) => n + (r.durMs ?? 0), 0)
      : null;
    runHits.push({ first, endAt: run[run.length - 1].end, count: run.length, totalMs });
    i = j + 1;
  }
}

function spanAt(mx: number, my: number): RunHit | null {
  const domain = Math.max(1e-6, t1 - t0);
  // sequence 模式优先 run 命中（与绘制一致）；duration 模式按单 span
  const candidates: { start: number; end: number; lane: number; hit: RunHit | null }[] =
    props.mode === "sequence"
      ? runHits.map((r) => ({ start: r.first.start, end: r.endAt, lane: r.first.lane, hit: r }))
      : spans.map((s) => ({ start: s.start, end: s.end, lane: s.lane, hit: null }));
  let fallback: Span | null = null;
  for (const c of candidates) {
    const y = TOP_PAD + c.lane * LANE_H;
    if (my < y - 1 || my > y + LANE_H + 1) continue;
    const x0 = ((c.start - t0) / domain) * width;
    const x1 = Math.max(x0 + 2, ((c.end - t0) / domain) * width);
    if (mx >= x0 - 2 && mx <= x1 + 2) {
      if (c.hit) return c.hit;
      // duration 模式：反查命中的单 span
      const s = spans.find((sp) => sp.start === c.start && sp.lane === c.lane);
      if (s) fallback = s;
    }
  }
  if (fallback) {
    // duration 模式包装成单元素 RunHit
    return { first: fallback, endAt: fallback.end, count: 1, totalMs: fallback.durMs };
  }
  return null;
}

function onMove(e: MouseEvent) {
  onDragMove(e); // 拖拽平移（dragX >= 0 时生效）
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const r = spanAt(e.clientX - rect.left, e.clientY - rect.top);
  tooltip.value = r
    ? {
        x: e.clientX - rect.left,
        y: e.clientY - rect.top,
        text:
          `#${r.first.seq}${r.count > 1 ? `~#${r.first.seq + r.count - 1}` : ""} ${r.first.label}${r.first.isError ? " · 错误" : ""}` +
          (r.count > 1 ? ` ×${r.count}` : "") +
          ` · ${r.first.timeText}` +
          (r.totalMs != null
            ? r.totalMs > 0
              ? ` · ${(r.totalMs / 1000).toFixed(1)}s`
              : " · 瞬时"
            : " · 耗时未记录"),
      }
    : null;
}

function onClick(e: MouseEvent) {
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const r = spanAt(e.clientX - rect.left, e.clientY - rect.top);
  if (r) emit("pick", r.first.seq + r.count - 1); // run 末条（最接近「这组刚结束」）
}

function onWheel(e: WheelEvent) {
  e.preventDefault();
  const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
  const mx = e.clientX - rect.left;
  const domain = Math.max(1e-6, t1 - t0);
  const ratio = Math.min(1, Math.max(0, mx / Math.max(1, width)));
  const anchor = t0 + ratio * domain;
  // dsh 的指数缩放；最小窗口：序号 4 槽 / 耗时 20ms；缩放不越出数据域
  const minWin = props.mode === "sequence" ? 4 : 20;
  const nd = Math.min(Math.max(domain * Math.exp(e.deltaY * 0.0015), minWin), d1 - d0);
  t0 = Math.min(Math.max(anchor - nd * ratio, d0), d1 - nd);
  t1 = t0 + nd;
  scheduleRedraw();
}

let dragX = -1;
const dragging = ref(false);
function onDragStart(e: MouseEvent) {
  if (e.button !== 0) return;
  dragX = e.clientX;
}
function onDragMove(e: MouseEvent) {
  if (dragX < 0) return;
  const dx = e.clientX - dragX;
  if (Math.abs(dx) < 2) return;
  dragX = e.clientX;
  dragging.value = true;
  const domain = Math.max(1e-6, t1 - t0);
  const dt = -(dx / Math.max(1, width)) * domain;
  t0 = Math.min(Math.max(t0 + dt, d0), d1 - domain);
  t1 = t0 + domain;
  scheduleRedraw();
}
function onDragEnd() {
  dragX = -1;
  dragging.value = false;
}

// ---- 生命周期 ----

watch([() => props.events, () => props.mode], () => {
  buildSpans();
  rebuildRunHits();
  scheduleRedraw();
});

/** live 追加后由父级调用：增量扩展了数据域，恢复用户原缩放/平移视域。
 *  保存比例（视口宽/域宽）与归一化位置，在新域上重建等比例视口——
 *  用户若已缩放到某段细节，新事件到达不打断其观察。 */
function preserveViewport() {
  const oldDomain = t1 - t0;
  if (oldDomain <= 0) return;
  const ratio = oldDomain / Math.max(1e-6, d1 - d0);
  const anchorNorm = (t0 - d0) / Math.max(1e-6, d1 - d0);
  const newDomain = (d1 - d0) * ratio;
  t0 = d0 + anchorNorm * (d1 - d0);
  t1 = t0 + newDomain;
  if (t1 > d1) {
    t1 = d1;
    t0 = Math.max(d0, t1 - newDomain);
  }
  scheduleRedraw();
}
defineExpose({ preserveViewport });
watch(() => props.selectedSeq, scheduleRedraw);
watch([() => props.hasEarlier, () => props.loadingEarlier], scheduleRedraw);

onMounted(() => {
  buildSpans();
  rebuildRunHits();
  if (track.value && typeof ResizeObserver !== "undefined") {
    ro = new ResizeObserver(scheduleRedraw);
    ro.observe(track.value);
  }
  scheduleRedraw();
});

onBeforeUnmount(() => {
  ro?.disconnect();
  if (redrawRaf) cancelAnimationFrame(redrawRaf);
});

const labels = computed(() => LANE_LABELS.map((text, i) => ({ text, active: laneActive.value[i] })));
</script>

<template>
  <div class="tt">
    <div class="tt-labels" aria-hidden="true">
      <span v-for="l in labels" :key="l.text" :class="{ dim: !l.active }">{{ l.text }}</span>
    </div>
    <div
      ref="track"
      class="tt-track"
      :class="{ 'tt-dragging': dragging }"
      @mousemove="onMove"
      @mouseleave="tooltip = null; onDragEnd()"
      @click="onClick"
      @wheel="onWheel"
      @mousedown="onDragStart"
      @mouseup="onDragEnd"
    >
      <canvas ref="canvas" class="tt-canvas" />
      <button
        v-if="earlierVisible"
        class="tt-earlier"
        :disabled="loadingEarlier"
        title="加载更早的事件"
        @mousedown.stop
        @click.stop="emit('load-earlier')"
      >…</button>
      <div v-if="tooltip" class="tt-tip" :style="{ left: `${Math.min(tooltip.x + 8, Math.max(0, width - 200))}px`, top: `${tooltip.y - 8}px` }">
        {{ tooltip.text }}
      </div>
      <div v-if="empty || !events.length" class="tt-empty">无时间轴数据</div>
    </div>
  </div>
</template>

<style scoped>
.tt {
  display: flex;
  height: 86px;
  flex-shrink: 0;
  border-bottom: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  user-select: none;
  overflow: hidden;
}

/* 左侧泳道标签列（英文短名 + 56px）：靠标签分区，泳道间不画分隔线。
   顶部 padding 与 canvas TOP_PAD 同值，每格高度 = LANE_H，与泳道逐像素对齐 */
.tt-labels {
  width: 56px;
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  padding-top: 6px;
  border-right: 1px solid var(--ip-color-border-default);
}
.tt-labels span {
  height: 16px;
  display: flex;
  align-items: center;
  justify-content: flex-end;
  padding-right: 8px;
  font-size: var(--ip-text-micro-size);
  letter-spacing: 0.3px;
  color: var(--ip-color-text-tertiary);
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.tt-labels span.dim { color: var(--ip-color-text-disabled); opacity: 0.55; }

.tt-track {
  position: relative;
  flex: 1;
  min-width: 0;
  cursor: crosshair;
  overflow: hidden;
}
.tt-canvas { display: block; }
.tt-dragging { cursor: grabbing; }

/* 左缘「…」：还有更早分页时的加载入口（视口贴住数据域起点时出现） */
.tt-earlier {
  position: absolute;
  left: 0;
  top: 0;
  bottom: 16px;
  width: 26px;
  z-index: 4;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  padding-left: 4px;
  border: none;
  outline: none;
  appearance: none;
  background: linear-gradient(to right, var(--ip-color-bg-secondary) 0, var(--ip-color-bg-secondary) 40%, transparent 100%);
  color: var(--ip-color-text-tertiary);
  font-size: 12px;
  cursor: pointer;
  opacity: 0.72;
}
.tt-earlier:hover { opacity: 1; color: var(--ip-color-text-primary); }
.tt-earlier:disabled { cursor: wait; }

.tt-tip {
  position: absolute;
  pointer-events: none;
  padding: 2px 8px;
  font-size: var(--ip-text-micro-size);
  font-family: var(--ip-font-mono, monospace);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  box-shadow: var(--ip-shadow-sm);
  white-space: nowrap;
  z-index: 5;
}
.tt-empty {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-disabled);
}
</style>

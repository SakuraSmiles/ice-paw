<!--
  TrajectoryTable — 轨迹主表（dsh inspection-ledger：紧凑事件表 + 虚拟滚动）

  行体系（固定行高，手写窗口化，天然扛几千轮）：
  - 列头 32px（吸顶）：类型 | 内容 | token·耗时
  - turn-header 40px：整行带（浅底、去 rail 去圆角；左缘内缩 8px，比子项贴边——
    分组容器语义；第 N 轮 · 日期·时间 · 终止 | ⚠错误 · 统计 · 耗时 · 用量），
    点击折叠/展开；折叠态 = 只留头
  - event      36px：比 turn 头再内缩一层（左 16px）；[KIND 徽章][单行摘要
    ellipsis][token/耗时]，点击选中 → 检查器；hover/选中 = 圆角底色填充
    （kind 语义由徽章承担，无侧边色条）

  对齐不变式：事件卡左内缩 16px + 内距 16px = 徽章起点 32px，与吸顶列头
  padding-left 逐像素对齐。「加载更早」前置行时滚动位置锚定不跳。
-->
<script setup lang="ts">
import { computed, ref, watch } from "vue";
import type { TrajectoryRow } from "../../composables/useTrajectory";
import { isWarnTermination, termLabel } from "../../utils/termLabels";

const props = defineProps<{
  rows: TrajectoryRow[];
  /** 选中的事件 seq（行高亮 + 时间轴联动） */
  selectedSeq: number | null;
  /** 选中的 turn 头 turnKey（轮次级检查器；行高亮） */
  selectedTurnKey: string | null;
  /** 搜索词非空时未命中行降透明度；命中行内高亮片段 */
  searching: boolean;
  /** 搜索词（searching 时行内高亮用；空串不切分） */
  searchQuery: string;
  hasMore: boolean;
  loadingEarlier: boolean;
  /** 选中行的 row key（跨会话合并流用：seq 是 per-conv 的，不同会话可同 seq
   *  → 按 seq 高亮会串行。传了本字段则优先按 key 精确匹配；单会话路径不传零变化） */
  selectedKey?: string | null;
  /** 会话元信息（跨会话合并流用：turn 头渲染会话名徽章）。key = session_id；
   *  不传 = 单会话路径零变化。turn 头的桶键是 `${session_id}::${turn_id}`
   *  前缀形态（useProjectTrajectory.scopeTurnKeys），按前缀反查会话 */
  sessionMeta?: Map<string, { title: string; kind: string }>;
}>();
const emit = defineEmits<{
  "select-row": [row: TrajectoryRow];
  "toggle-turn": [turnKey: string];
  "load-earlier": [];
}>();

const ROW_H: Record<TrajectoryRow["type"], number> = { "turn-header": 40, event: 36 };
const OVERSCAN = 12;

const scroller = ref<HTMLDivElement | null>(null);
const scrollTop = ref(0);
const viewportH = ref(600);

/** 每行起始偏移 + 总高（rows 变更时 O(n) 重算，纯前缀和） */
const layout = computed(() => {
  const offsets = new Array<number>(props.rows.length);
  let acc = 0;
  for (let i = 0; i < props.rows.length; i++) {
    offsets[i] = acc;
    acc += ROW_H[props.rows[i].type];
  }
  return { offsets, total: acc };
});

/** 可视窗口行切片（含 overscan）；offsets 有序 → 二分定位 */
const viewport = computed(() => {
  const { offsets, total } = layout.value;
  if (!props.rows.length) return { start: 0, end: 0, total };
  const top = scrollTop.value;
  const bottom = top + viewportH.value;
  let lo = 0;
  let hi = offsets.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (offsets[mid + 1] > top) hi = mid;
    else lo = mid + 1;
  }
  const start = Math.max(0, lo - OVERSCAN);
  let end = lo;
  while (end < offsets.length && offsets[end] < bottom) end++;
  return { start, end: Math.min(offsets.length, end + OVERSCAN), total };
});

const visibleRows = computed(() => {
  const { start, end } = viewport.value;
  const out: { row: TrajectoryRow; top: number; h: number }[] = [];
  for (let i = start; i < end; i++) {
    out.push({ row: props.rows[i], top: layout.value.offsets[i], h: ROW_H[props.rows[i].type] });
  }
  return out;
});

function onScroll() {
  if (!scroller.value) return;
  // 吸附状态机（对话页 useScrollFollow 同款纪律）：
  // 用户上滚（或被内容增长推离底部）→ 解除跟随；手动/内容增长回到贴底 → 重新吸附。
  // 仅在「用户主动滚动」时更新 pinned —— 程序性 scrollTo 用 programmatic 守卫跳过。
  const el = scroller.value;
  if (!programmatic) {
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < FOLLOW_THRESHOLD;
    pinned.value = nearBottom;
  }
  scrollTop.value = el.scrollTop;
  viewportH.value = el.clientHeight;
}
/** 底部吸附阈值：内容增长把视口推离底部 ≤2 行内仍视为贴底（36px 行高） */
const FOLLOW_THRESHOLD = 80;
const pinned = ref(true);
let programmatic = false;
function scrollToBottom() {
  if (!scroller.value) return;
  programmatic = true;
  scroller.value.scrollTop = layout.value.total;
  pinned.value = true;
  requestAnimationFrame(() => { programmatic = false; });
}

/** 时间轴/外部联动：滚到指定 seq 所在行 */
function scrollToSeq(seq: number) {
  const idx = props.rows.findIndex((r) => r.seq === seq);
  if (idx < 0 || !scroller.value) return;
  scroller.value.scrollTop = Math.max(0, layout.value.offsets[idx] - scroller.value.clientHeight / 3);
}

/** 键盘导航：滚到指定 turnKey 的轮次头 */
function scrollToTurn(turnKey: string) {
  const idx = props.rows.findIndex((r) => r.type === "turn-header" && r.turnKey === turnKey);
  if (idx < 0 || !scroller.value) return;
  scroller.value.scrollTop = Math.max(0, layout.value.offsets[idx] - scroller.value.clientHeight / 3);
}

/** 滚到指定 row key 所在行（跨会话合并流：seq 不唯一，按 key 定位） */
function scrollToKey(key: string) {
  const idx = props.rows.findIndex((r) => r.key === key);
  if (idx < 0 || !scroller.value) return;
  scroller.value.scrollTop = Math.max(0, layout.value.offsets[idx] - scroller.value.clientHeight / 3);
}

/** 是否贴近底部（外部跟随判据：pinned 状态机主导，此函数供首载等场景查询） */
function isNearBottom(threshold = FOLLOW_THRESHOLD): boolean {
  if (!scroller.value) return true;
  const el = scroller.value;
  return el.scrollHeight - el.scrollTop - el.clientHeight < threshold;
}

/** 平滑滚到底（live 追加跟随用；首载仍用 scrollToBottom 瞬跳） */
function smoothScrollToBottom() {
  if (!scroller.value) return;
  programmatic = true;
  pinned.value = true;
  scroller.value.scrollTo({ top: layout.value.total, behavior: "smooth" });
  requestAnimationFrame(() => { programmatic = false; });
}

/** 底部吸附中（sending 起新内容自动跟随；用户上滚解除） */
function isPinned(): boolean {
  return pinned.value;
}

// 「加载更早」前置行后保持视口稳定：prepend 前打标记，rows 变化后按高度差补偿 scrollTop
let pinPrepend = false;
function beginPrepend() {
  pinPrepend = true;
}
function totalOf(rs: TrajectoryRow[]): number {
  let a = 0;
  for (const r of rs) a += ROW_H[r.type];
  return a;
}
watch(
  () => props.rows,
  (nr, or) => {
    if (!scroller.value || !pinPrepend) {
      pinPrepend = false;
      return;
    }
    pinPrepend = false;
    const dh = totalOf(nr) - totalOf(or ?? []);
    if (dh > 0) scroller.value.scrollTop += dh;
  },
);
defineExpose({ scrollToSeq, scrollToTurn, scrollToKey, scrollToBottom, smoothScrollToBottom, isNearBottom, isPinned, beginPrepend });

// 终止原因文案：单一真相源在 utils/termLabels（词表外裸透原值）

function fmtTime(iso: string): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}:${String(d.getSeconds()).padStart(2, "0")}`;
}

function fmtDuration(ms: number | null): string {
  if (ms == null) return "";
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

function fmtTokens(n: number): string {
  return n >= 10000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

/** 事件行选中判定：跨会话流优先 row key（seq 跨会话可重复），单会话按 seq */
function isEventSelected(row: TrajectoryRow): boolean {
  if (row.type !== "event") return false;
  if (props.selectedKey != null) return row.key === props.selectedKey;
  return row.seq === props.selectedSeq;
}

/** turn 头的会话徽章（跨会话流）：桶键前缀 `${session_id}::` 反查；孤儿桶/查无 → null */
function sessionOfHeader(turnId: string | null): { title: string; kind: string } | null {
  if (!props.sessionMeta || !turnId) return null;
  const sid = turnId.split("::", 1)[0];
  return props.sessionMeta.get(sid) ?? null;
}

/** 搜索命中片段切分：把 summary 按 query（大小写不敏感）切成 [普通, 命中, …] 段 */
function splitHighlight(text: string): { text: string; hit: boolean }[] {
  const q = props.searching ? props.searchQuery.trim().toLowerCase() : "";
  if (!q) return [{ text, hit: false }];
  const out: { text: string; hit: boolean }[] = [];
  const lower = text.toLowerCase();
  let i = 0;
  for (;;) {
    const at = lower.indexOf(q, i);
    if (at < 0) break;
    if (at > i) out.push({ text: text.slice(i, at), hit: false });
    out.push({ text: text.slice(at, at + q.length), hit: true });
    i = at + q.length;
  }
  if (i < text.length) out.push({ text: text.slice(i), hit: false });
  return out;
}
</script>

<template>
  <div ref="scroller" class="ttab" @scroll.passive="onScroll">
    <div class="ttab-cols" aria-hidden="true">
      <span class="tc-kind">类型</span>
      <span class="tc-sum">内容</span>
      <span class="tc-metric">token · 耗时</span>
    </div>
    <div v-if="hasMore" class="ttab-earlier">
      <button class="ttab-earlier-btn" :disabled="loadingEarlier" @click="emit('load-earlier')">
        {{ loadingEarlier ? "加载中…" : "加载更早的事件" }}
      </button>
    </div>
    <div class="ttab-canvas" :style="{ height: `${viewport.total}px` }">
      <div
        v-for="item in visibleRows"
        :key="item.row.key"
        class="trow"
        :class="[`trow-${item.row.type}`, {
          selected: isEventSelected(item.row),
          'turn-selected': item.row.type === 'turn-header' && item.row.turnKey === selectedTurnKey,
          'turn-errored': item.row.type === 'turn-header' && item.row.errorCount > 0,
          dim: searching && item.row.type === 'event' && !item.row.match,
          streaming: item.row.type === 'event' && item.row.streaming,
        }]"
        :style="{ top: `${item.top}px` }"
        :title="item.row.type === 'turn-header' ? (item.row.collapsed ? '展开此轮' : '折叠此轮') : item.row.summary"
        @click="item.row.type === 'event' && !item.row.streaming ? emit('select-row', item.row) : item.row.type === 'turn-header' ? emit('toggle-turn', item.row.turnKey) : undefined"
      >
        <!-- turn 分割头：左（折叠箭头 + 会话徽章 + 轮次 + 日期·时间 + 终止）｜右（⚠ · 统计 · 耗时 · 用量） -->
        <template v-if="item.row.type === 'turn-header'">
          <span class="th-chevron">{{ item.row.collapsed ? "▸" : "▾" }}</span>
          <span
            v-if="sessionOfHeader(item.row.turnId)"
            class="th-session"
            :class="{ 'th-session-delegation': sessionOfHeader(item.row.turnId)!.kind === 'delegation' }"
            :title="sessionOfHeader(item.row.turnId)!.kind === 'delegation' ? '委派任务会话' : '对话会话'"
          >{{ sessionOfHeader(item.row.turnId)!.title }}</span>
          <span class="th-no">{{ item.row.turnId ? `第 ${item.row.turnIndex + 1} 轮` : "纪元前事件" }}</span>
          <span v-if="item.row.dateLabel" class="th-date">{{ item.row.dateLabel }}</span>
          <span class="th-time">{{ fmtTime(item.row.createdAt) }}</span>
          <span
            v-if="item.row.ended"
            class="th-term"
            :class="{ 'th-term-warn': isWarnTermination(item.row.ended.termination) }"
          >{{ termLabel(item.row.ended.termination) }}</span>
          <span v-else class="th-term th-term-pending">进行中</span>
          <span class="th-right">
            <span v-if="item.row.errorCount" class="th-err" title="本轮错误事件数">⚠ {{ item.row.errorCount }}</span>
            <span v-if="item.row.matchCount" class="th-match">{{ item.row.matchCount }} 命中</span>
            <span class="th-stats">{{ item.row.roundCount }} 条回复 · {{ item.row.toolCount }} 次工具</span>
            <span v-if="item.row.turnMs != null" class="th-usage">{{ fmtDuration(item.row.turnMs) }}</span>
            <span v-if="item.row.ended?.usage" class="th-usage" title="输入 / 输出 token">
              ↑{{ fmtTokens(item.row.ended.usage.prompt_tokens) }} ↓{{ fmtTokens(item.row.ended.usage.completion_tokens) }}
            </span>
            <!-- ⓘ 查看轮次详情：悬停显现（不抢常态视觉），点击选中头开检查器，不打断折叠 -->
            <button
              class="th-info"
              :class="{ visible: item.row.turnKey === selectedTurnKey }"
              title="查看本轮详情"
              @click.stop="emit('select-row', item.row)"
            >
              <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="12" cy="12" r="10" /><line x1="12" y1="16" x2="12" y2="12" /><line x1="12" y1="8" x2="12.01" y2="8" /></svg>
            </button>
          </span>
        </template>

        <!-- 事件行 -->
        <template v-else>
          <span class="ev-badge" :class="[`ev-${item.row.kind}`, { 'ev-err': item.row.isError }]" :title="item.row.event.kind">
            <span v-if="item.row.streaming" class="ev-pulse" />{{ item.row.label }}
          </span>
          <span class="ev-text" :class="{ 'ev-err-text': item.row.isError, 'ev-think-text': item.row.thinkingDerived }" :title="item.row.summary">
            <template v-for="(seg, si) in splitHighlight(item.row.summary)" :key="si">
              <mark v-if="seg.hit" class="ev-hit">{{ seg.text }}</mark>
              <template v-else>{{ seg.text }}</template>
            </template>
          </span>
          <span v-if="item.row.tokens != null" class="ev-tokens" title="token 计数">{{ fmtTokens(item.row.tokens) }} tok</span>
          <span v-if="item.row.durationMs != null" class="ev-dur" :title="item.row.kind === 'assistant' ? '生成耗时' : '执行耗时'">{{ fmtDuration(item.row.durationMs) }}</span>
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.ttab {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  overflow-x: hidden;
  position: relative;
}

/* 悬浮细滚动条（对齐 DevTools 质感） */
.ttab::-webkit-scrollbar { width: 11px; }
.ttab::-webkit-scrollbar-track { background: transparent; }
.ttab::-webkit-scrollbar-thumb {
  background: var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  border: 3px solid transparent;
  background-clip: padding-box;
}
.ttab::-webkit-scrollbar-thumb:hover { background-color: var(--ip-gray-400); }

/* ---- 吸顶列头：与事件行同栅格（内容起点 = 事件卡 16px 内缩 + 16px 内距）。
   11px 与表内 mono 指标列同级（chrome 微字号），低于内容层 13px ---- */
.ttab-cols {
  position: sticky;
  top: 0;
  z-index: 4;
  display: flex;
  align-items: center;
  gap: 12px;
  height: 32px;
  padding: 0 20px 0 32px;
  background: var(--ip-color-bg-secondary);
  border-bottom: 1px solid var(--ip-color-border-default);
  font-size: 11px;
  font-weight: var(--ip-font-weight-medium);
  letter-spacing: 0.3px;
  color: var(--ip-color-text-tertiary);
}
.tc-kind { flex-shrink: 0; min-width: 72px; }
.tc-sum { flex: 1; min-width: 0; }
.tc-metric { flex-shrink: 0; }

.ttab-earlier {
  position: sticky;
  top: 32px;
  z-index: 3;
  display: flex;
  justify-content: center;
  padding: 10px 0;
}
.ttab-earlier-btn {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  padding: 4px 18px;
  cursor: pointer;
  box-shadow: var(--ip-shadow-xs);
}
.ttab-earlier-btn:hover:not(:disabled) { background: var(--ip-color-bg-tertiary); }
.ttab-earlier-btn:disabled { opacity: 0.5; cursor: wait; }

.ttab-canvas { position: relative; width: 100%; }

.trow {
  position: absolute;
  left: 0;
  right: 0;
  display: flex;
  align-items: center;
  gap: 12px;
  overflow: hidden;
  cursor: pointer;
}

/* ---- turn 头：整行带（去 rail 去圆角），左缘比事件卡更贴边——折叠条是分组
   容器、子项是内容，贴边差 8px 即层级（整行宽度也强化"管辖全部子项"的语义） ---- */
.trow-turn-header {
  left: 8px;
  right: 8px;
  margin-top: 2px;
  height: 36px; /* 槽 40px：上下各留 2px 呼吸缝 */
  padding: 0 12px;
  background: var(--ip-color-bg-tertiary);
  font-size: var(--ip-text-caption-size);
  transition: background var(--ip-duration-fast) var(--ip-ease-out);
}
.trow-turn-header:hover { background: var(--ip-color-bg-elevated); }
.trow-turn-header:active { background: var(--ip-color-bg-secondary); }
/* 轮次头选中（检查器展示中）：底色与事件行选中态同语言 */
.trow-turn-header.turn-selected { background: var(--ip-color-selection-bg); }
/* 含错误轮次：淡红底扫读锚点（比 ⚠ 计数徽章更醒目）；hover 仍可辨 */
.trow-turn-header.turn-errored { background: var(--ip-danger-bg); }
.trow-turn-header.turn-errored:hover { background: var(--ip-danger-bg); filter: brightness(0.97); }
.trow-turn-header.turn-errored.turn-selected { background: var(--ip-color-selection-bg); }
.th-chevron { font-size: 9px; color: var(--ip-color-text-tertiary); width: 10px; flex-shrink: 0; }
/* 会话徽章（跨会话合并流）：委派会话走 tint 令牌（soft 系，勿直接 primary 底），
   chat 会话中性；max-width 兜长标题（title 悬停看全名） */
.th-session {
  max-width: 140px;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  font-size: 11px; line-height: 18px;
  padding: 0 8px; border-radius: var(--ip-radius-full);
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-secondary);
  flex-shrink: 0;
}
.th-session-delegation {
  color: var(--ip-color-primary-tint-text);
  background: var(--ip-color-primary-tint-bg);
}
.th-no { font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); white-space: nowrap; }
.th-date, .th-time {
  font-family: var(--ip-font-mono, monospace);
  font-size: 11px;
  color: var(--ip-color-text-disabled);
  white-space: nowrap;
  font-variant-numeric: tabular-nums;
}
.th-date {
  color: var(--ip-color-text-tertiary);
  padding: 1px 6px;
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm);
}
.th-term {
  padding: 1px 8px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  font-size: 11px;
}
.th-term-warn { color: var(--ip-warning-text); background: var(--ip-warning-bg); }
.th-term-pending { color: var(--ip-color-text-disabled); font-style: italic; }

.th-right { margin-left: auto; display: flex; align-items: center; gap: 12px; white-space: nowrap; }
/* ⓘ 轮次详情入口：默认隐没，悬停轮次头或已选中时可见 */
.th-info {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  border: none;
  background: none;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  border-radius: var(--ip-radius-full);
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out), background var(--ip-duration-fast) var(--ip-ease-out), color var(--ip-duration-fast) var(--ip-ease-out);
}
.trow-turn-header:hover .th-info,
.th-info.visible { opacity: 1; }
.th-info:hover { background: var(--ip-color-bg-elevated); color: var(--ip-primary-600); }
.th-stats { color: var(--ip-color-text-tertiary); }
.th-err {
  color: var(--ip-danger-text);
  background: var(--ip-danger-bg);
  padding: 1px 8px;
  border-radius: var(--ip-radius-full);
  font-size: 11px;
}
.th-match {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  padding: 1px 8px;
  border-radius: var(--ip-radius-full);
  font-size: 11px;
}
.th-usage {
  font-family: var(--ip-font-mono, monospace);
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}

/* ---- 事件行：比 turn 头再内缩一层（左 16px vs 头 8px），hover/选中 = 圆角底色
   填充。kind 语义由徽章承担，无侧边色条 ---- */
.trow-event {
  left: 16px;
  right: 8px;
  margin-top: 1px;
  height: 34px; /* 槽 36px：上下缝各 1px */
  padding: 0 16px;
  font-size: var(--ip-text-caption-size);
  border-radius: var(--ip-radius-md);
  transition: background var(--ip-duration-fast) var(--ip-ease-out);
}
.trow-event:hover { background: var(--ip-color-bg-tertiary); }
.trow-event.selected { background: var(--ip-color-selection-bg); }
.trow-event.dim { opacity: 0.28; }
/* 生成中 ephemeral 行：弱化底色 + 呼吸感（临时态，终态落库后被真实行取代） */
.trow-event.streaming { font-style: italic; color: var(--ip-color-text-secondary); }
.trow-event.streaming .ev-text { color: var(--ip-color-text-tertiary); }
/* 脉冲点（badge 内前置）：主题色呼吸动画 */
.ev-pulse {
  width: 5px;
  height: 5px;
  margin-right: 4px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-primary-500);
  animation: ev-pulse 1.2s ease-in-out infinite;
  flex-shrink: 0;
}
@keyframes ev-pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.35; transform: scale(0.8); }
}

.ev-badge {
  flex-shrink: 0;
  min-width: 72px;
  height: 18px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  font-size: 10px;
  font-weight: var(--ip-font-weight-semibold);
  letter-spacing: 0.5px;
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
}
.ev-user { color: var(--ip-info-text); background: var(--ip-info-bg); }
.ev-assistant { color: var(--ip-color-text-secondary); background: var(--ip-color-bg-tertiary); }
.ev-tool { color: var(--ip-warning-text); background: var(--ip-warning-bg); }
.ev-error { color: var(--ip-danger-text); background: var(--ip-danger-bg); }
.ev-discarded { color: var(--ip-color-text-disabled); background: var(--ip-color-bg-tertiary); }
.ev-summary { color: var(--ip-color-text-secondary); background: var(--ip-color-bg-tertiary); }
/* PLAN（计划快照）：成功色系——勾选清单的直觉语义 */
.ev-plan { color: var(--ip-success-text); background: var(--ip-success-bg); }
.ev-err { color: var(--ip-danger-text) !important; background: var(--ip-danger-bg) !important; }

/* 摘要文本列（勿名 ev-summary：那是 SUMMARY kind 徽章类）。
   body-sm 而非 caption：摘要是主阅读内容，36px 行高足够承载 */
.ev-text {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-body);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}
.ev-err-text { color: var(--ip-danger-base); }
/* 搜索命中片段：轻底高亮（mark 语义标签，默认黄底样式全覆写为主题语言） */
.ev-hit {
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  color: var(--ip-primary-700, var(--ip-primary-600));
  border-radius: 2px;
  padding: 0 1px;
}
/* 思考代摘要：斜体弱化——内心活动而非发言（全量思考在检查器） */
.ev-think-text { color: var(--ip-color-text-tertiary); font-style: italic; }
.ev-tokens, .ev-dur {
  flex-shrink: 0;
  font-family: var(--ip-font-mono, monospace);
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
</style>

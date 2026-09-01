<!--
  TrajectoryView — 轨迹回放主视图（会话「轨迹」标签页内容）

  dsh inspection-ledger 架构（Chrome DevTools 风）：
    [Toolbar 40px] 搜索 / 展开收起合一 / 辅助事件开关 / 导出 JSONL
    [Timeline 72px] canvas 瀑布图（点击联动表格行）
    [Table 虚拟行] 紧凑事件表 + [Inspector] 按需展现的局部检查器（选中行才渲染）

  数据：useTrajectory 尾部优先分页（最新 1000 条，「加载更早」向前翻）；
  行模型 buildRows 纯函数派生（折叠/搜索/辅助开关是视图状态）。
-->
<script setup lang="ts">
import { computed, nextTick, ref, watch, onMounted, onBeforeUnmount } from "vue";
import { listen } from "@tauri-apps/api/event";
import { buildRows, useTrajectory, type TrajectoryRow, type EventRow } from "../../composables/useTrajectory";
import { useResizablePanel } from "../../composables/useResizablePanel";
import type { SessionEvent } from "../../types";
import { bridge } from "../../api/bridge";
import { useChatStore } from "../../stores/chat";
import TrajectoryToolbar from "./TrajectoryToolbar.vue";
import TrajectoryTimeline from "./TrajectoryTimeline.vue";
import TrajectoryTable from "./TrajectoryTable.vue";
import TrajectoryInspector from "./TrajectoryInspector.vue";
import PanelResizeHandle from "../common/PanelResizeHandle.vue";

const props = defineProps<{
  conversationId: string;
  /** 所在 tab 是否激活（ChatPage 传入）：切到轨迹 tab = 想看最新状态 → 贴底 */
  active?: boolean;
}>();
const chat = useChatStore();
const { events, loading, loadingEarlier, error, legacy, hasMore, turnOffset, load, loadEarlier, refreshLatest } = useTrajectory();

// ---- 视图状态（行模型的派生输入） ----
const query = ref("");
const showAux = ref(false);
/** 时间轴投影：序号等宽（默认）/ 真实耗时 + 空闲压缩（dsh Duration 开关，同款持久化约定） */
const DURATION_KEY = "icepaw-traj-duration";
const durationMode = ref(localStorage.getItem(DURATION_KEY) === "1");
watch(durationMode, (v) => localStorage.setItem(DURATION_KEY, v ? "1" : "0"));
/** 折叠的 turn 集合（替换式更新保响应） */
const collapsedTurns = ref<Set<string>>(new Set());
const selectedRow = ref<TrajectoryRow | null>(null);

const searching = computed(() => query.value.trim() !== "");

// ---- 生成中 ephemeral 行：复用 chat store 流式状态（与对话页同源，零后端改动） ----
// 排序恒在落库行之后（seq=-1）；assistant 终态落库 + sending=false 后自然消失。
// 折叠态不显示（turn 头看不到子行）；搜索态不显示（match 语义混乱）。
const streamingRows = computed<TrajectoryRow[]>(() => {
  if (!chat.sending || chat.activeConvId !== props.conversationId) return [];
  if (query.value.trim() !== "") return []; // 搜索态 match 语义混乱，不显示
  const out: TrajectoryRow[] = [];
  const tk = "__streaming__";
  const now = new Date().toISOString();
  const mk = (kind: EventRow["kind"], label: string, summary: string, key: string): EventRow => ({
    type: "event",
    key,
    turnKey: tk,
    seq: -1,
    createdAt: now,
    t: null,
    kind,
    label,
    summary,
    isError: false,
    thinkingDerived: false,
    durationMs: null,
    tokens: null,
    match: true,
    streaming: true,
    // 占位事件（渲染/检查器均不读 ephemeral 行 payload；点击也被守卫拦截）
    event: {
      id: -1,
      session_id: "",
      seq: -1,
      kind: "assistant_message",
      actor: "agent",
      turn_id: null,
      message_id: null,
      payload: {},
      created_at: now,
    } as SessionEvent,
  });
  const think = chat.streamingThinking;
  if (think) {
    const line = think.split("\n", 1)[0].trim().slice(0, 120);
    out.push(mk("assistant", "THINKING", line || "思考中…", "st-thinking"));
  }
  const text = chat.streamingText;
  if (text) {
    const line = text.split("\n", 1)[0].trim().slice(0, 120);
    out.push(mk("assistant", "STREAM", line || "生成中…", "st-text"));
  }
  for (const call of chat.streamingToolCalls.values()) {
    out.push(mk("tool", "TOOL", `${call.name} ${call.ended ? "· 等待结果" : "· 组装参数"}…`, `st-tool-${call.id}`));
  }
  return out;
});

const rows = computed(() =>
  streamingRows.value.length
    ? [...buildRows(events.value, { collapsedTurns: collapsedTurns.value, showAux: showAux.value, query: query.value, turnOffset: turnOffset.value }), ...streamingRows.value]
    : buildRows(events.value, { collapsedTurns: collapsedTurns.value, showAux: showAux.value, query: query.value, turnOffset: turnOffset.value }),
);

/** 会话级汇总（工具栏 chip）：轮数含窗口前偏移（全局值）；事件/工具为已载窗口内计数 */
const stats = computed(() => {
  let turns = 0;
  let tools = 0;
  let lastTk: string | null = null;
  for (const ev of events.value) {
    const tk = ev.turn_id ?? "__orphan__";
    if (tk !== lastTk) {
      turns += 1;
      lastTk = tk;
    }
    if (ev.kind === "tool_execution") tools += 1;
  }
  return { turns: turns + turnOffset.value, events: events.value.length, tools };
});
/** 搜索态下是否至少命中一行（全未命中时表格上方浮提示；ephemeral 行 match 恒 true 不算未命中） */
const anyMatch = computed(() => rows.value.some((r) => r.type === "event" && r.match));

const turnKeys = computed(() => {
  const keys = new Set<string>();
  for (const ev of events.value) keys.add(ev.turn_id ?? "__orphan__");
  return keys;
});

function toggleTurn(turnKey: string) {
  const next = new Set(collapsedTurns.value);
  if (next.has(turnKey)) next.delete(turnKey);
  else next.add(turnKey);
  collapsedTurns.value = next;
}

function expandAll() {
  collapsedTurns.value = new Set();
}

function collapseAll() {
  collapsedTurns.value = new Set(turnKeys.value);
}

/** 展开/收起合一：有任一折叠 → 展开全部；全展开 → 收起全部 */
const anyCollapsed = computed(() => collapsedTurns.value.size > 0);
function toggleTurns() {
  if (anyCollapsed.value) expandAll();
  else collapseAll();
}

// ---- 选中（再点同一行 = 取消选中收起检查器） ----
function selectRow(row: TrajectoryRow) {
  // 按 key 判同（T-2：live 追加/搜索后重派生，身份比较失配致再点同行无法取消选中）
  selectedRow.value = selectedRow.value?.key === row.key ? null : row;
}

// ---- 检查器宽度：useResizablePanel 共享机制（UX #2 规范化，原手搓版迁移） ----
// 注意：换了存储 key（icepaw-traj-insp-width → icepaw-panel-traj-inspector），
// 老用户存过的宽度弃用一次（回默认 420），不值得为一次性迁移写兼容读。
const {
  width: inspWidth,
  dragging: inspDragging,
  startDrag: onInspResizeStart,
  reset: resetInspWidth,
} = useResizablePanel({ key: "traj-inspector", default: 420, min: 300, max: 720, dir: -1 });

// ---- 表格 ↔ 时间轴联动 ----
const tableRef = ref<InstanceType<typeof TrajectoryTable> | null>(null);
const toolbarRef = ref<InstanceType<typeof TrajectoryToolbar> | null>(null);
const selectedSeq = computed(() => (selectedRow.value?.type === "event" ? selectedRow.value.seq : null));
/** 选中的是 turn 头时，表格侧高亮该折叠条（轮次级检查器打开中） */
const selectedTurnKey = computed(() => (selectedRow.value?.type === "turn-header" ? selectedRow.value.turnKey : null));

function pickFromTimeline(seq: number) {
  const row = rows.value.find((r) => r.seq === seq);
  if (row) selectedRow.value = row;
  tableRef.value?.scrollToSeq(seq);
}

// ---- 键盘导航（DevTools 习惯）：↑/↓ 移动选中（含轮次头，可开轮次检查器）·
//      Enter = 打开详情（turn 头选中时）· 空格 = 折叠所选行所在轮 · Esc 关检查器 ·
//      / 聚焦搜索 · 搜索框 Enter/Shift+Enter 命中行间循环跳转 ----
/** ↑/↓ 在全部行（turn 头 + 事件）上移动选中——轮次头也可被选中开检查器 */
function moveSelection(dir: 1 | -1) {
  if (!rows.value.length) return;
  const curIdx = rows.value.findIndex((r) => r === selectedRow.value);
  const idx = curIdx < 0 ? (dir === 1 ? 0 : rows.value.length - 1) : Math.min(rows.value.length - 1, Math.max(0, curIdx + dir));
  selectedRow.value = rows.value[idx];
  void nextTick(() => {
    const r = rows.value[idx];
    if (r.type === "event") tableRef.value?.scrollToSeq(r.seq);
    else tableRef.value?.scrollToTurn(r.turnKey);
  });
}

function onKeydown(e: KeyboardEvent) {
  const t = e.target as HTMLElement | null;
  const editing = !!t && (t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable);
  if (e.key === "Escape") {
    if (!editing) selectedRow.value = null;
    return;
  }
  if (editing || e.ctrlKey || e.metaKey || e.altKey) return;
  if (e.key === "ArrowDown" || e.key === "ArrowUp") {
    e.preventDefault();
    moveSelection(e.key === "ArrowDown" ? 1 : -1);
  } else if (e.key === "/") {
    e.preventDefault();
    toolbarRef.value?.focusSearch();
  } else if (e.key === "Enter" && selectedRow.value) {
    e.preventDefault();
    // turn 头选中时 Enter = 打开详情（内容即检查器）；事件行已选中，无额外动作
  } else if (e.key === " " && selectedRow.value) {
    e.preventDefault();
    toggleTurn(selectedRow.value.turnKey);
  }
}

// ---- 搜索跳转：Enter/Shift+Enter 在命中行间循环（以选中行为锚，无选中从头开始） ----
function searchJump(dir: 1 | -1) {
  const hits = rows.value.filter((r): r is Extract<TrajectoryRow, { type: "event" }> => r.type === "event" && r.match);
  if (!hits.length) return;
  const curSeq = selectedSeq.value;
  const cur = curSeq == null ? -1 : hits.findIndex((r) => r.seq === curSeq);
  let idx: number;
  if (cur < 0) idx = dir === 1 ? 0 : hits.length - 1;
  else if (cur + dir >= hits.length) idx = 0; // 循环
  else if (cur + dir < 0) idx = hits.length - 1;
  else idx = cur + dir;
  selectedRow.value = hits[idx];
  void nextTick(() => tableRef.value?.scrollToSeq(hits[idx].seq));
}

// ---- 导出 JSONL ----
const exporting = ref(false);
const exportMsg = ref<string | null>(null);
let exportTimer: ReturnType<typeof setTimeout> | null = null;

async function exportJsonl() {
  exporting.value = true;
  exportMsg.value = null;
  try {
    const path = await bridge.trajectory.exportJsonl(props.conversationId);
    exportMsg.value = `已导出：${path}`;
  } catch (e) {
    exportMsg.value = `导出失败：${e instanceof Error ? e.message : String(e)}`;
  } finally {
    exporting.value = false;
    if (exportTimer) clearTimeout(exportTimer);
    exportTimer = setTimeout(() => { exportMsg.value = null; }, 6000);
  }
}

// ---- 加载（v-show 保持挂载，靠 watch conversationId 驱动刷新） ----
function resetViewState() {
  query.value = "";
  showAux.value = false;
  collapsedTurns.value = new Set();
  selectedRow.value = null;
}

/** 首次载入滚到底部：尾部优先分页下，底部 = 会话最新状态 */
async function loadAndScroll(id: string) {
  await load(id);
  await nextTick();
  tableRef.value?.scrollToBottom();
}

/** 「加载更早」前置行：先打锚，rows 更新后表格按高度差补偿 scrollTop（视口不跳） */
function loadEarlierStable() {
  tableRef.value?.beginPrepend();
  void loadEarlier();
}

onMounted(() => void loadAndScroll(props.conversationId));
watch(() => props.conversationId, (id) => {
  resetViewState();
  void loadAndScroll(id);
});

// tab 激活即贴底：进轨迹 tab = 想看最新状态（live 台账的心智）。数据由常开的
// push 监听保持新鲜，这里只管滚到底。无 immediate——首次进入由 onMounted
// loadAndScroll 负责（visibility 叠放下隐藏期布局有效，挂载滚底本就生效）。
watch(() => props.active, (v) => {
  if (!v) return;
  void nextTick(() => tableRef.value?.scrollToBottom());
});

// ---- live 追加（v2 事件驱动 + 兜底轮询）----
// 后端 append_event 落库成功即广播 → lib.rs 转 Tauri event「session:event-appended」
// 推前端；本视图按 conversation_id 过滤后拉增量（list_after，已载 max_seq 作游标）。
// 通知在落库之后发出，到达时行必可查，无竞态；增量拉取幂等（重复通知拉 0 条）。
// POLL_MS 是兜底而非主路径（webview 忙时 event 交付延迟 / emit 失败等极端情形）。
const POLL_MS = 5000;
let pollTimer: ReturnType<typeof setTimeout> | null = null;
let polling = false;
let unlistenAppended: (() => void) | null = null;

const timelineRef = ref<InstanceType<typeof TrajectoryTimeline> | null>(null);

async function pollOnce() {
  const n = await refreshLatest();
  if (n > 0) {
    timelineRef.value?.preserveViewport();
    if (tableRef.value?.isPinned()) {
      await nextTick();
      tableRef.value?.smoothScrollToBottom();
    }
  }
}

async function startPolling() {
  if (polling) return;
  polling = true;
  try {
    await pollOnce();
  } finally {
    polling = false;
  }
  pollTimer = setTimeout(startPolling, POLL_MS);
}

function stopPolling() {
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = null;
}

/** sending 结束后再补一轮（终态事件 turn_ended 与最后一条 assistant 落库时序贴近）。
 *  sending 起始 = 用户刚发消息 → 强制贴底进入跟随态（意图明确）。 */
watch(() => chat.sending, (sending) => {
  if (sending) {
    tableRef.value?.scrollToBottom();
    startPolling();
  } else {
    stopPolling();
    void pollOnce();
  }
});

// 事件通知监听（挂载期常开）：本会话有新事件落库 → 即时拉增量。sending 期之外
// 也监听——为将来多 agent 后台事件铺路（现在等价覆盖：事件只在 sending 期产生）。
onMounted(() => {
  void listen<{ conversation_id: string; kind: string }>("session:event-appended", (e) => {
    if (e.payload.conversation_id === props.conversationId) void pollOnce();
  }).then((u) => { unlistenAppended = u; });
});
onBeforeUnmount(() => {
  stopPolling();
  unlistenAppended?.();
  unlistenAppended = null;
});
</script>

<template>
  <div class="trajectory-view" tabindex="-1" @keydown="onKeydown">
    <TrajectoryToolbar
      ref="toolbarRef"
      v-model:query="query"
      v-model:show-aux="showAux"
      v-model:duration-mode="durationMode"
      :any-collapsed="anyCollapsed"
      :stats="stats"
      :exporting="exporting"
      @toggle-turns="toggleTurns"
      @search-jump="searchJump"
      @export="exportJsonl"
    />
    <TrajectoryTimeline
      ref="timelineRef"
      :events="events"
      :selected-seq="selectedSeq"
      :mode="durationMode ? 'duration' : 'sequence'"
      :turn-offset="turnOffset"
      :has-earlier="hasMore"
      :loading-earlier="loadingEarlier"
      @pick="pickFromTimeline"
      @load-earlier="loadEarlierStable"
    />

    <div class="traj-main">
      <div class="traj-table-wrap">
        <div v-if="loading" class="traj-empty">加载中…</div>
        <div v-else-if="error" class="traj-empty traj-error">加载失败：{{ error }}</div>
        <div v-else-if="legacy" class="traj-empty">此会话无事件日志（早于事件纪元），无法回放轨迹。</div>
        <div v-else-if="rows.length === 0" class="traj-empty">暂无事件。</div>
        <template v-else>
          <Transition name="overlay">
            <div v-if="searching && !anyMatch" class="traj-nohit">无命中事件</div>
          </Transition>
          <TrajectoryTable
            ref="tableRef"
            :rows="rows"
            :selected-seq="selectedSeq"
            :selected-turn-key="selectedTurnKey"
            :searching="searching"
            :search-query="query"
            :has-more="hasMore"
            :loading-earlier="loadingEarlier"
            @select-row="selectRow"
            @toggle-turn="toggleTurn"
            @load-earlier="loadEarlierStable"
          />
        </template>
      </div>
      <!-- 检查器按需展现：选中行才渲染（无空态占位），✕/Esc/再点同一行 = 取消选中即收起 -->
      <template v-if="selectedRow">
        <PanelResizeHandle
          flow="inline"
          :class="{ 'insp-resizing': inspDragging }"
          @dragstart="onInspResizeStart"
          @reset="resetInspWidth"
        />
        <TrajectoryInspector :row="selectedRow" :style="{ width: `${inspWidth}px` }" @close="selectedRow = null" />
      </template>
    </div>

    <Transition name="overlay">
      <div v-if="exportMsg" class="traj-toast">{{ exportMsg }}</div>
    </Transition>
  </div>
</template>

<style scoped>
.trajectory-view {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: var(--ip-color-bg-secondary);
}

.traj-main { flex: 1; display: flex; min-height: 0; }
.traj-table-wrap { flex: 1; display: flex; position: relative; min-width: 0; min-height: 0; }
/* 底缘渐隐：与对话页同款（输入区上边线已撤，分区由内容渐隐承担——见
   ChatMessages .messages-wrap::after）。只蒙事件表列：右侧检查器是带边框
   的自有面板，不吃渐隐。 */
.traj-table-wrap::after {
  content: '';
  position: absolute;
  left: 0; right: 0; bottom: 0;
  height: 72px;
  background: linear-gradient(to bottom,
    transparent,
    color-mix(in srgb, var(--ip-color-bg-secondary) 32%, transparent) 48%,
    color-mix(in srgb, var(--ip-color-bg-secondary) 74%, transparent) 78%,
    var(--ip-color-bg-secondary));
  pointer-events: none;
  z-index: 1;
}
.trajectory-view:focus { outline: none; }

.traj-nohit {
  position: absolute;
  top: 44px;
  left: 50%;
  transform: translateX(-50%);
  z-index: 6;
  padding: 4px 16px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  box-shadow: var(--ip-shadow-sm);
  pointer-events: none;
}

/* 检查器把手拖拽中：热区亮线持续显形（指针快速甩出把手时 :hover 会掉，
   用 dragging 态兜住——类落在 PanelResizeHandle 根元素上，父 scoped 可直选） */
.insp-resizing::after {
  opacity: 0.65;
  transform: translateX(-50%) scaleY(1);
}

.traj-empty { flex: 1; display: flex; align-items: center; justify-content: center; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-tertiary); }
.traj-empty.traj-error { color: var(--ip-danger-base); }

.traj-toast {
  position: absolute;
  top: 40px;
  right: 16px;
  z-index: 10;
  padding: 8px 14px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-md);
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  max-width: 60vw;
  word-break: break-all;
}

.overlay-enter-active { animation: traj-fade 0.2s ease-out; }
.overlay-leave-active { animation: traj-fade 0.15s ease-in reverse; }
@keyframes traj-fade { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }
</style>

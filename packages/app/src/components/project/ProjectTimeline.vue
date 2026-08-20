<!--
  ProjectTimeline — 项目轨迹视图（MA-2）：项目内全部会话的事件按全局 id 合并成
  一条因果流（跨会话可比的只有 session_events 全局自增 id；seq 是 per-conv 的）。

  复用面（D7）：TrajectoryTable（虚拟行）+ TrajectoryInspector 原样复用；
  buildRows 零改动，调用前 scopeTurnKeys 纯适配（turn_id 加 session 前缀，
  防跨会话错误合桶）。不复用 TrajectoryToolbar/Timeline——会话域语义
  （导出/瀑布图是单会话的），本项目自带 slim 控件。

  v1 边缘（接受并记录）：两会话并发流式时同一轮被切成多个头（全局 id 交错），
  头统计是全量预扫信息不丢，只是分段展示；瀑布图/per-agent 泳道待独立设计。
-->
<script setup lang="ts">
import { computed, nextTick, onActivated, onMounted, ref, watch } from "vue";
import { buildRows, type TrajectoryRow } from "../../composables/useTrajectory";
import { scopeTurnKeys, useProjectTrajectory } from "../../composables/useProjectTrajectory";
import { useResizablePanel } from "../../composables/useResizablePanel";
import { useChatStore } from "../../stores/chat";
import TrajectoryTable from "../trajectory/TrajectoryTable.vue";
import TrajectoryInspector from "../trajectory/TrajectoryInspector.vue";
import PanelResizeHandle from "../common/PanelResizeHandle.vue";

const props = defineProps<{ projectId: string }>();
const chat = useChatStore();

// 项目会话集谓词（live 通知过滤）：chat store 的 project_id 缓存（侧栏常驻加载）。
// 直链进入时 store 可能未拉——此时已载会话集兜底（有事件的会话必在其中）。
function isProjectConv(convId: string): boolean {
  return chat.conversations.some((c) => c.id === convId && c.project_id === props.projectId);
}

const { events, loading, loadingEarlier, error, hasMore, load, loadEarlier, refreshLatest } =
  useProjectTrajectory(() => props.projectId, isProjectConv);

// ---- 视图状态（行模型的派生输入；与 TrajectoryView 同款语义） ----
const query = ref("");
const showAux = ref(false);
const collapsedTurns = ref<Set<string>>(new Set());
const selectedRow = ref<TrajectoryRow | null>(null);
const searching = computed(() => query.value.trim() !== "");

// ---- 行模型：scopeTurnKeys 适配 → buildRows 零改动复用 ----
// turnOffset 恒 0：跨会话无全局轮序，「第 N 轮」弱化为窗口内段序号（M3 的
// 全局轮号查询是单会话语义，项目轴不适用）。
const rows = computed(() =>
  buildRows(scopeTurnKeys(events.value), {
    collapsedTurns: collapsedTurns.value,
    showAux: showAux.value,
    query: query.value,
    turnOffset: 0,
  }),
);

/** 会话徽章元信息（Table sessionMeta prop）：从事件流自带的两列去重收敛 */
const sessionMeta = computed(() => {
  const m = new Map<string, { title: string; kind: string }>();
  for (const e of events.value) {
    if (!m.has(e.session_id)) m.set(e.session_id, { title: e.session_title, kind: e.session_kind });
  }
  return m;
});

const anyMatch = computed(() => rows.value.some((r) => r.type === "event" && r.match));

function toggleTurn(turnKey: string) {
  const next = new Set(collapsedTurns.value);
  if (next.has(turnKey)) next.delete(turnKey);
  else next.add(turnKey);
  collapsedTurns.value = next;
}
const anyCollapsed = computed(() => collapsedTurns.value.size > 0);
function toggleTurns() {
  collapsedTurns.value = anyCollapsed.value ? new Set() : new Set(rows.value.filter((r) => r.type === "turn-header").map((r) => r.turnKey));
}

// ---- 选中（再点同一行 = 取消选中收起检查器）；跨会话流按 row key 联动 ----
const selectedKey = computed(() => (selectedRow.value?.type === "event" ? selectedRow.value.key : null));
const selectedTurnKey = computed(() => (selectedRow.value?.type === "turn-header" ? selectedRow.value.turnKey : null));

/** 按 row.key 判同（buildRows 每次求值产新对象，身份判等在重派生后会失配） */
function selectRow(row: TrajectoryRow) {
  selectedRow.value = selectedRow.value?.key === row.key ? null : row;
}

// ---- 检查器宽度：与单会话轨迹视图共享存储 key（同一使用偏好） ----
const {
  width: inspWidth,
  dragging: inspDragging,
  startDrag: onInspResizeStart,
  reset: resetInspWidth,
} = useResizablePanel({ key: "traj-inspector", default: 420, min: 300, max: 720, dir: -1 });

// ---- 表格编排 ----
const tableRef = ref<InstanceType<typeof TrajectoryTable> | null>(null);
const searchRef = ref<HTMLInputElement | null>(null);

/** 首次载入滚到底部：尾部优先分页下，底部 = 项目最新状态 */
async function loadAndScroll() {
  await load();
  await nextTick();
  tableRef.value?.scrollToBottom();
}

/** 「加载更早」前置行：先打锚，rows 更新后表格按高度差补偿 scrollTop（视口不跳） */
function loadEarlierStable() {
  tableRef.value?.beginPrepend();
  void loadEarlier();
}

onMounted(() => void loadAndScroll());
// 常规路径不触发（DetailLayout 的 keep-alive component 按 :key=项目 id 重建实例），
// 仅直链热替换等边缘场景兜底刷新
watch(() => props.projectId, () => { void loadAndScroll(); });

// ---- 键盘（DevTools 习惯的子集）：↑/↓ 移动选中 · Esc 关检查器/清搜索 · / 聚焦 ----
function moveSelection(dir: 1 | -1) {
  if (!rows.value.length) return;
  // 按 key 判同（T-1：rows 是 computed，live 追加/搜索后重派生产新对象，身份比较失配致 ↑↓ 跳首尾）
  const curIdx = rows.value.findIndex((r) => r.key === selectedRow.value?.key);
  const idx = curIdx < 0 ? (dir === 1 ? 0 : rows.value.length - 1) : Math.min(rows.value.length - 1, Math.max(0, curIdx + dir));
  selectedRow.value = rows.value[idx];
  void nextTick(() => {
    const r = rows.value[idx];
    if (r.type === "event") tableRef.value?.scrollToKey(r.key);
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
    searchRef.value?.focus();
  } else if (e.key === " " && selectedRow.value) {
    e.preventDefault();
    toggleTurn(selectedRow.value.turnKey);
  }
}

/** Enter/Shift+Enter 在命中行间循环（key 锚定——seq 跨会话可重复） */
function searchJump(dir: 1 | -1) {
  const hits = rows.value.filter((r): r is Extract<TrajectoryRow, { type: "event" }> => r.type === "event" && r.match);
  if (!hits.length) return;
  const curKey = selectedKey.value;
  const cur = curKey == null ? -1 : hits.findIndex((r) => r.key === curKey);
  let idx: number;
  if (cur < 0) idx = dir === 1 ? 0 : hits.length - 1;
  else if (cur + dir >= hits.length) idx = 0;
  else if (cur + dir < 0) idx = hits.length - 1;
  else idx = cur + dir;
  selectedRow.value = hits[idx];
  void nextTick(() => tableRef.value?.scrollToKey(hits[idx].key));
}

/** live 追加跟随：事件增长（composable 的通知驱动 refreshLatest 已拼接）后，
 *  贴底时平滑滚——与单会话轨迹同款纪律（不重复拉取，只管滚） */
async function followIfPinned() {
  if (!tableRef.value?.isPinned()) return;
  await nextTick();
  tableRef.value?.smoothScrollToBottom();
}
watch(() => events.value.length, (n, o) => {
  if (n > o) void followIfPinned();
});

// keep-alive 离开期间错过的通知由补拉兜底（D6：事件驱动为主，无常驻轮询）
onActivated(async () => {
  const n = await refreshLatest();
  if (n > 0) await followIfPinned();
});
</script>

<template>
  <div class="ptimeline" tabindex="-1" @keydown="onKeydown">
    <!-- slim 控件：搜索 / 展开收起 / 辅助事件（会话域的导出·耗时开关不搬） -->
    <div class="pt-bar">
      <div class="pt-search" title="按 / 快速聚焦；Enter/Shift+Enter 在命中行间跳转">
        <svg class="pt-search-icon" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><circle cx="11" cy="11" r="7" /><line x1="21" y1="21" x2="16.5" y2="16.5" /></svg>
        <input
          ref="searchRef"
          type="text"
          placeholder="跨会话搜索内容、参数、结果…"
          :value="query"
          @input="query = ($event.target as HTMLInputElement).value"
          @keydown.esc.stop.prevent="query = ''"
          @keydown.enter.stop.prevent="searchJump($event.shiftKey ? -1 : 1)"
        />
        <button v-if="query" class="pt-search-clear" title="清空（Esc）" @click="query = ''">✕</button>
      </div>

      <button class="pt-pill-btn" :class="{ folded: anyCollapsed }" :title="anyCollapsed ? '展开所有轮次' : '折叠所有轮次'" @click="toggleTurns">
        {{ anyCollapsed ? "展开全部" : "收起全部" }}
      </button>

      <label class="pt-toggle" title="附件落库 / 视觉适配 / 钩子注入等低频事件">
        <input type="checkbox" :checked="showAux" @change="showAux = ($event.target as HTMLInputElement).checked" />
        <span class="pt-pill">辅助事件</span>
      </label>

      <div class="pt-spacer" />
      <span class="pt-hint">项目内 {{ sessionMeta.size }} 个会话按事件时序合并</span>
    </div>

    <div class="pt-main">
      <div class="pt-table-wrap">
        <div v-if="loading" class="pt-empty">加载中…</div>
        <div v-else-if="error" class="pt-empty pt-error">加载失败：{{ error }}</div>
        <div v-else-if="rows.length === 0" class="pt-empty">项目内还没有事件——创建会话或委派任务后，事件会在此按时间线汇总。</div>
        <template v-else>
          <Transition name="pt-overlay">
            <div v-if="searching && !anyMatch" class="pt-nohit">无命中事件</div>
          </Transition>
          <TrajectoryTable
            ref="tableRef"
            :rows="rows"
            :selected-seq="null"
            :selected-key="selectedKey"
            :selected-turn-key="selectedTurnKey"
            :searching="searching"
            :search-query="query"
            :has-more="hasMore"
            :loading-earlier="loadingEarlier"
            :session-meta="sessionMeta"
            @select-row="selectRow"
            @toggle-turn="toggleTurn"
            @load-earlier="loadEarlierStable"
          />
        </template>
      </div>
      <!-- 检查器按需展现：选中行才渲染（TrajectoryInspector 原样复用） -->
      <template v-if="selectedRow">
        <PanelResizeHandle
          flow="inline"
          :class="{ 'pt-resizing': inspDragging }"
          @dragstart="onInspResizeStart"
          @reset="resetInspWidth"
        />
        <TrajectoryInspector :row="selectedRow" :style="{ width: `${inspWidth}px` }" @close="selectedRow = null" />
      </template>
    </div>
  </div>
</template>

<style scoped>
.ptimeline {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
  min-height: 0;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  background: var(--ip-color-bg-secondary);
  overflow: hidden;
}
.ptimeline:focus { outline: none; }

/* ---- slim 控件条（对齐 TrajectoryToolbar 的控件语言，去掉会话域项） ---- */
.pt-bar {
  display: flex;
  align-items: center;
  gap: 12px;
  height: 40px;
  padding: 0 14px;
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}
.pt-search {
  display: flex;
  align-items: center;
  gap: 6px;
  width: 260px;
  height: 26px;
  padding: 0 9px;
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out), background var(--ip-duration-fast) var(--ip-ease-out);
}
.pt-search:focus-within { border-color: var(--ip-color-border-focus); background: var(--ip-color-bg-primary); }
.pt-search-icon { color: var(--ip-color-text-tertiary); flex-shrink: 0; }
.pt-search input {
  flex: 1; min-width: 0; height: 100%;
  border: none; outline: none; background: transparent;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-primary);
}
.pt-search input::placeholder { color: var(--ip-color-text-placeholder); }
.pt-search-clear {
  border: none; background: none; padding: 0 2px;
  font-size: 10px; color: var(--ip-color-text-tertiary); cursor: pointer;
  border-radius: var(--ip-radius-full); flex-shrink: 0;
}
.pt-search-clear:hover { color: var(--ip-color-text-primary); }

.pt-pill-btn {
  height: 26px;
  display: inline-flex;
  align-items: center;
  padding: 0 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  cursor: pointer;
  white-space: nowrap;
  transition: var(--ip-transition-colors);
}
.pt-pill-btn:hover { color: var(--ip-color-text-primary); background: var(--ip-color-bg-elevated); }
.pt-pill-btn.folded {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  border-color: var(--ip-primary-200);
}

.pt-toggle { display: flex; align-items: center; cursor: pointer; user-select: none; }
.pt-toggle input { position: absolute; opacity: 0; pointer-events: none; }
.pt-pill {
  height: 26px;
  display: inline-flex;
  align-items: center;
  padding: 0 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full);
  white-space: nowrap;
  transition: var(--ip-transition-colors);
}
.pt-toggle:hover .pt-pill { color: var(--ip-color-text-primary); }
.pt-toggle input:checked + .pt-pill {
  color: var(--ip-primary-600);
  background: var(--ip-color-primary-soft-bg, var(--ip-primary-50));
  border-color: var(--ip-primary-200);
}

.pt-spacer { flex: 1; }
.pt-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); white-space: nowrap; }

.pt-main { flex: 1; display: flex; min-height: 0; }
.pt-table-wrap { flex: 1; display: flex; position: relative; min-width: 0; min-height: 0; }

.pt-empty { flex: 1; display: flex; align-items: center; justify-content: center; font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-tertiary); }
.pt-empty.pt-error { color: var(--ip-danger-base); }

.pt-nohit {
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

/* 检查器把手拖拽中亮线持续显形（把手 :hover 快速甩出会掉，dragging 态兜住） */
.pt-resizing::after { opacity: 0.65; transform: translateX(-50%) scaleY(1); }

.pt-overlay-enter-active { animation: pt-fade 0.2s ease-out; }
.pt-overlay-leave-active { animation: pt-fade 0.15s ease-in reverse; }
@keyframes pt-fade { from { opacity: 0; transform: translateY(-4px); } to { opacity: 1; transform: translateY(0); } }
</style>

<script setup lang="ts">
// TaskPanel.vue — 会话级任务胶囊 + popover（MA-1 UX，C3）
//
// 本会话派生任务（kind='delegation' 且 parent=本会话）的唯一索引入口：
// 消息流里的委派卡片是「就地锚点」（读到哪看到哪），这里是全量索引——
// 不用翻消息流找那个还在跑的任务。点击行 → 任务详情（子会话，直落轨迹 tab）。
//
// 取数全前端派生（conversations + streamingConvIds），零后端。
// 排序律（已拍板）：状态优先（进行中置顶）+ 时间倒序；不分页——本会话任务
// 天然有限（深度=1、串行），超 MAX_ROWS 截断并提示进项目页看全量。
// 状态两态：进行中（脉冲）/已结束（中性点）——done/failed 精确终态是 MA-2
// 台账（turn_ended 派生状态机）的事，此处不伪造。
import { computed, ref, watch, onBeforeUnmount } from "vue";
import { useChatStore } from "../../stores/chat";
import { formatTime } from "../../utils/time";

const chat = useChatStore();

interface TaskRow {
  id: string;
  title: string;
  running: boolean;
  updatedAt: number;
}

const tasks = computed<TaskRow[]>(() => {
  const pid = chat.activeConvId;
  if (!pid) return [];
  return chat.conversations
    .filter((c) => c.kind === "delegation" && c.parent_conversation_id === pid)
    .map((c) => ({
      id: c.id,
      title: c.title || "委派任务",
      running: chat.streamingConvIds.has(c.id),
      updatedAt: new Date(c.updated_at).getTime() || 0,
    }))
    .sort((a, b) => (a.running === b.running ? b.updatedAt - a.updatedAt : a.running ? -1 : 1));
});

const MAX_ROWS = 8;
const visibleTasks = computed(() => tasks.value.slice(0, MAX_ROWS));
const anyRunning = computed(() => tasks.value.some((t) => t.running));

// ---- popover 开合（点击外部关闭；切会话收起——数据源已变） ----
const open = ref(false);
const panelRef = ref<HTMLElement | null>(null);

function onDocClick(e: MouseEvent) {
  if (open.value && panelRef.value && !panelRef.value.contains(e.target as Node)) {
    open.value = false;
  }
}
watch(open, (v) => {
  if (v) document.addEventListener("click", onDocClick);
  else document.removeEventListener("click", onDocClick);
});
onBeforeUnmount(() => document.removeEventListener("click", onDocClick));
watch(() => chat.activeConvId, () => { open.value = false; });

function openTask(id: string) {
  open.value = false;
  chat.openConversationAtTrajectory(id);
}
</script>

<template>
  <!-- 无任务时零占用（胶囊是索引不是状态栏；计划段（C5）存在时也会点亮胶囊） -->
  <div v-if="tasks.length > 0" ref="panelRef" class="task-panel">
    <button
      class="task-pill"
      :class="{ open }"
      :title="anyRunning ? '本会话的任务（有进行中）' : '本会话的任务'"
      @click.stop="open = !open"
    >
      <span class="task-pill-dot" :class="{ running: anyRunning }" />
      <span>任务</span>
      <span class="task-pill-count">{{ tasks.length }}</span>
    </button>

    <Transition name="dropdown">
      <div v-if="open" class="task-popover" @click.stop>
        <div class="task-popover-title">任务（本会话）</div>
        <button v-for="t in visibleTasks" :key="t.id" class="task-row" @click="openTask(t.id)">
          <span class="task-dot" :class="{ running: t.running }" />
          <span class="task-row-title" :title="t.title">{{ t.title }}</span>
          <span class="task-row-time">{{ formatTime(new Date(t.updatedAt).toISOString()) }}</span>
        </button>
        <div v-if="tasks.length > MAX_ROWS" class="task-more">
          还有 {{ tasks.length - MAX_ROWS }} 个，全部任务请在项目页查看
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.task-panel { position: relative; margin-left: auto; display: flex; align-items: center; }

/* 胶囊：与 tab 同排右侧（ChatPage 标签条），计数 + 进行中脉冲 */
.task-pill {
  display: flex; align-items: center; gap: 6px;
  padding: 4px 12px; border-radius: var(--ip-radius-full, 999px);
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-secondary);
  font-size: var(--ip-text-caption-size); cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.task-pill:hover { color: var(--ip-color-text-primary); border-color: var(--ip-primary-400); }
.task-pill.open { color: var(--ip-primary-600); border-color: var(--ip-primary-400); background: var(--ip-primary-soft-bg, rgba(46,141,100,0.08)); }
.task-pill-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: var(--ip-color-text-tertiary); }
.task-pill-dot.running { background: var(--ip-warning-base, #d97706); animation: task-pulse 1.2s ease-in-out infinite; }
.task-pill-count { font-weight: var(--ip-font-weight-medium); }
@keyframes task-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

/* popover：胶囊正下展开 */
.task-popover {
  position: absolute; top: calc(100% + 6px); right: 0; z-index: 100;
  min-width: 300px; max-width: 380px; padding: 6px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  display: flex; flex-direction: column; gap: 2px;
}
.task-popover-title {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  padding: 4px 10px 6px;
}
.task-row {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 7px 10px; border: none; border-radius: var(--ip-radius-md);
  background: transparent; cursor: pointer; text-align: left;
  transition: background var(--ip-duration-fast) var(--ip-ease-out);
}
.task-row:hover { background: var(--ip-color-bg-tertiary); }
.task-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: var(--ip-color-text-tertiary); }
.task-dot.running { background: var(--ip-warning-base, #d97706); animation: task-pulse 1.2s ease-in-out infinite; }
.task-row-title {
  flex: 1; min-width: 0; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary);
}
.task-row-time { flex-shrink: 0; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.task-more { padding: 6px 10px 4px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.dropdown-enter-active { animation: task-drop 0.15s ease-out; }
.dropdown-leave-active { animation: task-drop 0.1s ease-in reverse; }
@keyframes task-drop {
  from { opacity: 0; transform: translateY(-4px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
</style>

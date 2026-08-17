<script setup lang="ts">
// TaskPanel.vue — 会话级任务胶囊 + 双栏 popover（MA-1 UX，C3 + C5 计划段，P12 重设计）
//
// 本会话派生任务（kind='delegation' 且 parent=本会话）的唯一索引入口：
// 消息流里的委派卡片是「就地锚点」（读到哪看到哪），这里是全量索引——
// 不用翻消息流找那个还在跑的任务。点击行 → 任务详情（子会话，直落轨迹 tab）。
//
// 取数：任务全前端派生（conversations + streamingConvIds）；计划走
// get_session_plan（最后一条 plan_updated 全量快照），切会话/开面板/
// session:event-appended(kind=plan_updated) 三处刷新。
// 排序律（已拍板）：状态优先（进行中置顶）+ 时间倒序；不分页——本会话任务
// 天然有限（深度=1、串行），P12 起全量进列内滚动（不再 MAX_ROWS 截断）。
// 状态两态：进行中（脉冲）/已结束（中性点）——done/failed 精确终态是 MA-2
// 台账（turn_ended 派生状态机）的事，此处不伪造。计划勾选同理恒为 agent 判断。
import { computed, ref, watch, onMounted, onBeforeUnmount } from "vue";
import { listen } from "@tauri-apps/api/event";
import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { formatTime, parseDbTime } from "../../utils/time";
import { bridge } from "../../api/bridge";
import type { PlanSnapshot } from "../../types";

const chat = useChatStore();
const agentStore = useAgentStore();

/** 旧数据「委派: 」前缀展示侧归一剥离（UX #4：新生成标题已无前缀，零 migration）*/
function delegationTitle(raw: string): string {
  return raw.replace(/^委派:\s*/, "") || "委派任务";
}

interface TaskRow {
  id: string;
  title: string;
  /** 被委派的专家 agent 名（行内标识；agent 已删则空，隐藏不占位）*/
  agentName: string | null;
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
      title: delegationTitle(c.title || "委派任务"),
      agentName: c.agent_id ? agentStore.getById(c.agent_id)?.name ?? null : null,
      running: chat.streamingConvIds.has(c.id),
      // DB 时间串是 UTC（"YYYY-MM-DD HH:MM:SS" 无时区标记），必须走 parseDbTime
      // （补 Z）——裸 new Date() 会当本地时间解析，UTC+8 下全部慢 8 小时
      updatedAt: parseDbTime(c.updated_at).getTime() || 0,
    }))
    .sort((a, b) => (a.running === b.running ? b.updatedAt - a.updatedAt : a.running ? -1 : 1));
});

const anyRunning = computed(() => tasks.value.some((t) => t.running));

// ---- 规模治理（用户拍板 2026-08-17）：胶囊面板是轻量索引，不做全量浏览 ----
// 任务截断：running 全显（排序已置顶），非 running 最多 6 条，超出收计数行；
// 全量浏览将来归 MA-2 项目台账（出口先不空头挂链接）。
const MAX_DONE_TASK_ROWS = 6;
const visibleTasks = computed<TaskRow[]>(() => {
  const running = tasks.value.filter((t) => t.running);
  const rest = tasks.value.filter((t) => !t.running).slice(0, MAX_DONE_TASK_ROWS);
  return [...running, ...rest];
});
const hiddenTaskCount = computed(() => tasks.value.length - visibleTasks.value.length);

// ---- 计划段（C5）：get_session_plan 快照；null = 无计划/已清空 → 列隐藏 ----
const plan = ref<PlanSnapshot | null>(null);
const planDone = computed(() => plan.value?.items.filter((i) => i.status === "done").length ?? 0);

// 计划 done 折叠（规模治理，协议上限 30 条且推进后期 done 常占大头）：活跃条目
// 原序展开（agent 的意图顺序不打乱），done 收「已完成 N」折叠行。行绑定携带
// 原下标 i —— flash 键（plan-{i}）与折叠重排解耦。
const planDoneExpanded = ref(false);
const activePlanItems = computed(() =>
  (plan.value?.items ?? []).map((it, i) => ({ it, i })).filter(({ it }) => it.status !== "done"),
);
const donePlanItems = computed(() =>
  (plan.value?.items ?? []).map((it, i) => ({ it, i })).filter(({ it }) => it.status === "done"),
);
const planPct = computed(() => {
  const total = plan.value?.items.length ?? 0;
  return total > 0 ? Math.round((planDone.value / total) * 100) : 0;
});
const planActive = computed(() => plan.value?.items.some((i) => i.status === "in_progress") ?? false);

async function loadPlan() {
  const cid = chat.activeConvId;
  if (!cid) { plan.value = null; return; }
  try {
    plan.value = await bridge.trajectory.currentPlan(cid);
  } catch { plan.value = null; } // 拉取失败静默（下次触发点重试）
}

/** 胶囊可见性：有任务或有计划（计划是意图文档，先于任何委派存在） */
const panelVisible = computed(() => tasks.value.length > 0 || plan.value !== null);
/** 胶囊文案（P12）：任务与计划并列成组——「任务 N · 计划 D/M」，缺谁省谁 */
const pillLabel = computed(() => {
  const parts: string[] = [];
  if (tasks.value.length > 0) parts.push(`任务 ${tasks.value.length}`);
  if (plan.value) parts.push(`计划 ${planDone.value}/${plan.value.items.length}`);
  return parts.join(" · ");
});
/** 胶囊 dot 脉冲：任务在跑或计划推进中，都算「有进行中」 */
const anyActive = computed(() => anyRunning.value || planActive.value);

// ---- 状态变更轻闪（P12）：任务 running 翻转 / 计划条目 status 翻转时，
// 对应行加 .just-changed 背景轻闪 ~1s 后自动褪去——subtle，不弹跳。
// flashKeys 整体替换（Set 原地改不触发响应式）；卸载清定时器。
const flashKeys = ref<ReadonlySet<string>>(new Set());
const flashTimers = new Map<string, ReturnType<typeof setTimeout>>();

function flash(key: string) {
  if (flashKeys.value.has(key)) return;
  const next = new Set(flashKeys.value);
  next.add(key);
  flashKeys.value = next;
  clearTimeout(flashTimers.get(key));
  flashTimers.set(key, setTimeout(() => {
    const after = new Set(flashKeys.value);
    after.delete(key);
    flashKeys.value = after;
    flashTimers.delete(key);
  }, 1100));
}
watch(tasks, (list, prev) => {
  const before = new Map((prev ?? []).map((t) => [t.id, t.running]));
  for (const t of list) {
    const was = before.get(t.id);
    if (was !== undefined && was !== t.running) flash(t.id);
  }
});
watch(plan, (p, prev) => {
  if (!p || !prev) return; // 无→有 / 有→无无对应行，不闪
  p.items.forEach((it, i) => {
    const old = prev.items[i];
    if (old && old.status !== it.status) flash(`plan-${i}`);
  });
});
onBeforeUnmount(() => { flashTimers.forEach(clearTimeout); flashTimers.clear(); });

// ---- popover 开合（点击外部关闭；切会话收起——数据源已变） ----
const open = ref(false);
const panelRef = ref<HTMLElement | null>(null);

// ---- 自动展开（UX #3）：新委派任务启动时展开一次（用户拍板时机） ----
// 触发条件是「任务进入 running」而非「任务出现」：delegation-started 刷新列表时
// 子会话还没流出首个 chunk（running=false），若只看新增任务，seen 记账会在
// running 到来前把任务吞掉——popover 永不自动展开（手测 #4 根因）。
// autoOpenedIds 独立记账：切会话时把「已 running」的种子进去（切入时已在跑的
// 不算「新启动」，不弹）；每任务只自动展开一次，用户手动关后不追弹。
const seenTaskIds = new Set<string>();
const autoOpenedIds = new Set<string>();
let seenConvId: string | null = null;
watch(tasks, (list) => {
  const cid = chat.activeConvId;
  const freshConv = cid !== seenConvId;
  if (freshConv) {
    seenTaskIds.clear();
    autoOpenedIds.clear();
    seenConvId = cid;
    // 切入快照：已在跑的任务视为「已展开过」，否则首个新事件就会追弹
    list.forEach((t) => { if (t.running) autoOpenedIds.add(t.id); });
  }
  list.forEach((t) => seenTaskIds.add(t.id));
  if (freshConv) return; // 切会话首快照只记账不弹（防每次切入都弹，打扰）
  const candidate = list.find((t) => t.running && !autoOpenedIds.has(t.id));
  if (candidate) {
    autoOpenedIds.add(candidate.id);
    if (!open.value) open.value = true;
  }
}, { immediate: true });

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

// 切会话：收起 + 计划随数据源切换重载（任务 computed 自更新）+ done 折叠态复位
watch(() => chat.activeConvId, () => {
  open.value = false;
  planDoneExpanded.value = false;
  void loadPlan();
}, { immediate: true });
// 开面板时重拉（关闭期间可能有过更新——事件只在面板所在会话活跃时被消费）
watch(open, (v) => { if (v) void loadPlan(); });

// live：本会话有新 plan_updated 即刷新（附录：通知在落库后发出，到达时必可查）
let unlistenAppended: (() => void) | null = null;
onMounted(async () => {
  unlistenAppended = await listen<{ conversation_id: string; kind: string }>(
    "session:event-appended",
    (e) => {
      if (e.payload.kind === "plan_updated"
        && e.payload.conversation_id === chat.activeConvId) {
        void loadPlan();
      }
    },
  );
});
onBeforeUnmount(() => { unlistenAppended?.(); unlistenAppended = null; });

function openTask(id: string) {
  open.value = false;
  chat.openConversationAtTrajectory(id);
}
</script>

<template>
  <!-- 无任务且无计划时零占用（胶囊是索引不是状态栏） -->
  <div v-if="panelVisible" ref="panelRef" class="task-panel">
    <button
      class="task-pill"
      :class="{ open }"
      :title="anyActive ? '本会话的任务与计划（有进行中）' : '本会话的任务与计划'"
      @click.stop="open = !open"
    >
      <span class="task-pill-dot" :class="{ running: anyActive }" />
      <span v-if="pillLabel" class="task-pill-label">{{ pillLabel }}</span>
    </button>

    <Transition name="dropdown">
      <div v-if="open" class="task-popover" :class="{ 'has-plan': !!plan }" @click.stop>
        <!-- 双栏（P12）：左任务 / 右计划，各列独立滚动——溢出收敛进面板不出页面；
             窄窗 flex-wrap 自动堆叠回纵向 -->
        <div class="task-columns">
          <!-- 左列：任务 -->
          <div class="task-col">
            <div class="col-head">
              <span>任务</span>
              <span class="col-count">{{ tasks.length }}</span>
            </div>
            <div class="col-body">
              <TransitionGroup name="task-fade" tag="div" class="col-body-inner">
                <button
                  v-for="t in visibleTasks"
                  :key="t.id"
                  :class="['task-row', { 'just-changed': flashKeys.has(t.id) }]"
                  @click="openTask(t.id)"
                >
                  <span class="task-dot" :class="{ running: t.running }" />
                  <span class="task-row-title" :title="t.title">{{ t.title }}</span>
                  <span v-if="t.agentName" class="task-row-agent">{{ t.agentName }}</span>
                  <span class="task-row-time">{{ formatTime(new Date(t.updatedAt).toISOString()) }}</span>
                </button>
              </TransitionGroup>
              <div v-if="hiddenTaskCount > 0" class="task-more">还有 {{ hiddenTaskCount }} 个任务</div>
              <div v-if="tasks.length === 0" class="task-more">暂无委派任务</div>
            </div>
          </div>

          <!-- 右列：计划（意图文档全量快照；勾选恒为 agent 判断，非任务终态派生） -->
          <div v-if="plan" class="task-col">
            <div class="col-head plan-title">
              <span>计划 {{ planDone }}/{{ plan.items.length }}</span>
              <!-- 细进度条：一眼读完成度，不与条目状态标记抢语义 -->
              <span class="plan-progress" :title="`完成 ${planDone}/${plan.items.length}`">
                <span class="plan-progress-fill" :style="{ width: `${planPct}%` }" />
              </span>
            </div>
            <div class="col-body">
              <!-- 活跃条目（pending/in_progress）原序展开；done 收折叠行（规模治理） -->
              <div
                v-for="{ it, i } in activePlanItems"
                :key="i"
                :class="['task-plan-row', it.task_conversation_id ? 'task-plan-link' : '', { 'just-changed': flashKeys.has(`plan-${i}`) }]"
                :title="it.task_conversation_id ? '此步骤挂有委派任务，点击打开' : it.text"
                @click="it.task_conversation_id && openTask(it.task_conversation_id)"
              >
                <span :class="['plan-mark', `plan-mark-${it.status}`]" />
                <span class="task-row-title">{{ it.text }}</span>
                <span v-if="it.task_conversation_id" class="plan-jump" title="打开对应任务">↗</span>
              </div>
              <button v-if="donePlanItems.length > 0" class="plan-done-toggle" @click="planDoneExpanded = !planDoneExpanded">
                <span class="plan-mark plan-mark-done" />
                <span class="plan-done-label">已完成 {{ donePlanItems.length }}</span>
                <span class="plan-done-caret" :class="{ expanded: planDoneExpanded }">{{ planDoneExpanded ? "收起" : "展开" }}</span>
              </button>
              <!-- done 展开区：行内容与活跃区同构（改动需两处同步） -->
              <div
                v-for="{ it, i } in donePlanItems"
                v-show="planDoneExpanded"
                :key="`done-${i}`"
                :class="['task-plan-row', 'task-plan-done', it.task_conversation_id ? 'task-plan-link' : '', { 'just-changed': flashKeys.has(`plan-${i}`) }]"
                :title="it.task_conversation_id ? '此步骤挂有委派任务，点击打开' : it.text"
                @click="it.task_conversation_id && openTask(it.task_conversation_id)"
              >
                <span :class="['plan-mark', `plan-mark-${it.status}`]" />
                <span class="task-row-title">{{ it.text }}</span>
                <span v-if="it.task_conversation_id" class="plan-jump" title="打开对应任务">↗</span>
              </div>
            </div>
          </div>
        </div>
      </div>
    </Transition>
  </div>
</template>

<style scoped>
.task-panel { position: relative; margin-left: auto; display: flex; align-items: center; }

/* 胶囊：与 tab 同排右侧（ChatPage 标签条），任务·计划组文案 + 进行中脉冲 */
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
.task-pill-label { font-weight: var(--ip-font-weight-medium); }
@keyframes task-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

/* popover：胶囊正下展开；双栏 + 列内滚动（P12 溢出治理——多任务/多计划
   不再撑出页面，各列独立 max-height 内滚；窄窗 wrap 堆叠回纵向）。
   ⚠️ 显式定宽（不用 min/max-width 内容自适应）：容器宽由内容决定时，
   flex-basis:0 的两列实际宽度受 min-width 钳制，内容不够宽就会被 wrap
   挤成上下堆叠（用户手测踩过：220→260 后并排阈值 526 超过内容宽）。
   定宽后双栏恒并排：有计划 = 720 两列各 ~357，无计划 = 420 单列不空 */
.task-popover {
  position: absolute; top: calc(100% + 6px); right: 0; z-index: 100;
  width: min(420px, calc(100vw - 48px)); padding: 6px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
}
.task-popover.has-plan { width: min(720px, calc(100vw - 48px)); }
.task-columns { display: flex; flex-wrap: wrap; gap: 6px; }
.task-col { flex: 1 1 0; min-width: 260px; display: flex; flex-direction: column; gap: 2px; }
.col-head {
  display: flex; align-items: center; gap: 8px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  padding: 4px 10px 6px;
}
.col-count { font-weight: var(--ip-font-weight-medium); color: var(--ip-color-text-secondary); }
.col-body {
  max-height: min(320px, 48vh); overflow-y: auto;
  display: flex; flex-direction: column; gap: 2px; padding-right: 2px;
}
.col-body-inner { display: flex; flex-direction: column; gap: 2px; }
/* 计划标题行：文案 + 细进度条同行（完成度一眼可读） */
.plan-title { justify-content: space-between; }
.plan-progress { flex: 1; max-width: 120px; height: 3px; border-radius: 2px; background: var(--ip-color-bg-tertiary); overflow: hidden; }
.plan-progress-fill { display: block; height: 100%; border-radius: 2px; background: var(--ip-primary-500); transition: width var(--ip-duration-fast) var(--ip-ease-out); }
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
/* 被委派的专家名（UX #4）：小徽标式弱化呈现，不与任务文本抢主信息 */
.task-row-agent {
  flex-shrink: 0; max-width: 120px; overflow: hidden; white-space: nowrap; text-overflow: ellipsis;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  padding: 1px 8px; border-radius: var(--ip-radius-full, 999px);
  background: var(--ip-color-bg-tertiary);
}
.task-more { padding: 6px 10px 4px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* 计划条目行（与 PlanCard 同款状态标记，非按钮——仅挂任务的条目可点） */
.task-plan-row {
  display: flex; align-items: center; gap: 8px;
  padding: 6px 10px; border-radius: var(--ip-radius-md);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.task-plan-link { cursor: pointer; }
.task-plan-link:hover { background: var(--ip-color-bg-tertiary); }

/* done 折叠行（规模治理）：历史噪音的收纳口，caption 级弱化呈现 */
.plan-done-toggle {
  display: flex; align-items: center; gap: 8px; width: 100%;
  padding: 6px 10px; border: none; border-radius: var(--ip-radius-md);
  background: transparent; cursor: pointer; text-align: left;
  font-family: inherit; font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.plan-done-toggle:hover { background: var(--ip-color-bg-tertiary); }
.plan-done-caret { margin-left: auto; color: var(--ip-color-text-disabled); }
.plan-done-caret.expanded { color: var(--ip-color-text-tertiary); }
.plan-mark {
  width: 8px; height: 8px; flex-shrink: 0; border-radius: 50%;
  border: 1.5px solid var(--ip-color-text-tertiary);
  transition: background-color 0.25s var(--ip-ease-out), border-color 0.25s var(--ip-ease-out);
}
.plan-mark-in_progress { border-color: var(--ip-warning-base, #d97706); background: var(--ip-warning-base, #d97706); animation: task-pulse 1.2s ease-in-out infinite; }
.plan-mark-done { border-color: var(--ip-success-base, #16a34a); background: var(--ip-success-base, #16a34a); }
.task-plan-row .plan-mark-done + .task-row-title { text-decoration: line-through; color: var(--ip-color-text-tertiary); }
.plan-jump { flex-shrink: 0; font-size: var(--ip-text-caption-size); color: var(--ip-primary-500); }

/* 状态变更轻闪（P12）：soft-bg 底色 ~1s 褪去（prefers-reduced-motion 由全局兜底归零） */
.just-changed { animation: row-flash 0.9s var(--ip-ease-out); }
@keyframes row-flash {
  from { background-color: var(--ip-primary-soft-bg, rgba(46,141,100,0.12)); }
  to { background-color: transparent; }
}

/* 任务行新增淡入（enter-only；不做 move/leave——重排少且勿弹跳） */
.task-fade-enter-active { transition: opacity 0.3s var(--ip-ease-out); }
.task-fade-enter-from { opacity: 0; }

.dropdown-enter-active { animation: task-drop 0.15s ease-out; }
.dropdown-leave-active { animation: task-drop 0.1s ease-in reverse; }
@keyframes task-drop {
  from { opacity: 0; transform: translateY(-4px) scale(0.97); }
  to { opacity: 1; transform: translateY(0) scale(1); }
}
</style>

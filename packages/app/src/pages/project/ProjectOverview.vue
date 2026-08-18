<script setup lang="ts">
// ProjectOverview.vue — 项目详情「概览」tab（2026-08-18 重设计）：
//   统计带（4 数字卡：会话/委派任务/消息/成员 + 任务状态宽卡：五桶堆叠条）
//   + 会话入口（最近 5 条，点击回首页打开；完整列表在侧栏——「查看全部」
//   切 scope 回首页）。
// 成员/更新时间从头部移入此页（头部最简骨架只留名+简介）；任务台账 v1 表格
// 撤下（简陋），区域留待重新设计（数据仍在：统计卡分桶 + 轨迹 tab 全量）。
// live：useProjectTasks 事件驱动（delegation-started / turn_ended 去抖）；
// keep-alive 离开期间错过的事件由 onActivated 补拉兜底。
import { computed, onActivated, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { bridge } from "../../api/bridge";
import { useProjectTasks } from "../../composables/useProjectTasks";
import { useChatStore } from "../../stores/chat";
import { useProjectStore } from "../../stores/project";
import { useAgentStore } from "../../stores/agent";
import { taskStatus, TASK_STATUS_LABELS } from "../../utils/taskStatus";
import { parseDbTime, timeAgo } from "../../utils/time";
import type { ProjectOverview as OverviewData, Conversation } from "../../types";

const route = useRoute();
const router = useRouter();
const chat = useChatStore();
const project = useProjectStore();
const agent = useAgentStore();

const projectId = computed(() => String(route.params.id ?? ""));
const overview = ref<OverviewData | null>(null);

const { tasks, load: loadTasks, refresh } = useProjectTasks(projectId);

async function loadOverview() {
  try {
    overview.value = await bridge.projects.getOverview(projectId.value);
  } catch {
    overview.value = null; // 静默：统计带显示占位 —
  }
}

watch(projectId, () => {
  void loadOverview();
  void loadTasks();
}, { immediate: true });

onActivated(() => {
  void loadOverview();
  refresh();
});

// ---- 会话入口：本项目最近 5 条（chat store 由常驻 Sidebar 预载） ----
const RECENT_LIMIT = 5;

const projectConvs = computed<Conversation[]>(() => {
  const pid = projectId.value;
  return chat.conversations
    .filter((c) => c.project_id === pid && (!c.kind || c.kind === "chat"))
    .sort((a, b) => parseDbTime(b.updated_at).getTime() - parseDbTime(a.updated_at).getTime());
});
const recentConvs = computed(() => projectConvs.value.slice(0, RECENT_LIMIT));

// 相对时间每分钟刷新（Sidebar nowTick 同款机制）
const nowTick = ref(Date.now());
let tick: ReturnType<typeof setInterval> | null = null;
onMounted(() => { tick = setInterval(() => (nowTick.value = Date.now()), 60000); });
onUnmounted(() => { if (tick) clearInterval(tick); });

function ago(iso: string): string {
  return timeAgo(iso, nowTick.value);
}

/** 打开会话：回首页 + scope 同步到本项目（侧栏列表立即对上）+ 选中 */
function openConv(conv: Conversation) {
  project.setActiveProject(projectId.value);
  chat.selectConversation(conv.id);
  router.push("/");
}

/** 查看全部：scope 切到本项目回首页（完整列表在侧栏；选最近一条，与侧栏
 *  切换空间的行为一致） */
function viewAll() {
  project.setActiveProject(projectId.value);
  const first = projectConvs.value[0];
  if (first) chat.selectConversation(first.id);
  else chat.clearActiveConversation();
  router.push("/");
}

/** 成员卡点击 → 设置 tab（成员管理所在） */
function gotoMembers() {
  router.push(`/projects/${projectId.value}/settings`);
}

const memberCount = computed(() => project.getById(projectId.value)?.agents?.length ?? 0);

const lastActivityLabel = computed(() => {
  const at = overview.value?.last_activity_at;
  return at ? ago(at) : "";
});

// ---- 任务状态宽卡：五桶（running overlay 后的实时视角） ----
const bucketKeys = ["running", "done", "failed", "interrupted", "ended-other"] as const;
const buckets = computed(() => {
  const b = { running: 0, done: 0, failed: 0, interrupted: 0, "ended-other": 0 };
  for (const t of tasks.value) b[taskStatus(t, chat.streamingConvIds)] += 1;
  return b;
});
const taskTotal = computed(() => tasks.value.length);

const agentName = (id: string) => agent.getById(id)?.name ?? "未知";
</script>

<template>
  <div class="overview">
    <!-- ===== 统计带：4 数字卡 + 任务状态宽卡 ===== -->
    <div class="stat-band">
      <div class="stat-card">
        <span class="stat-icon">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" /></svg>
        </span>
        <div class="stat-body">
          <span class="stat-value">{{ overview?.chat_conversations ?? "—" }}</span>
          <span class="stat-label">会话</span>
        </div>
      </div>
      <div class="stat-card">
        <span class="stat-icon">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="22" y1="2" x2="11" y2="13" /><polygon points="22 2 15 22 11 13 2 9 22 2" /></svg>
        </span>
        <div class="stat-body">
          <span class="stat-value">{{ overview?.delegation_conversations ?? "—" }}</span>
          <span class="stat-label">委派任务</span>
        </div>
      </div>
      <div class="stat-card">
        <span class="stat-icon">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" /><polyline points="14 2 14 8 20 8" /><line x1="16" y1="13" x2="8" y2="13" /><line x1="16" y1="17" x2="8" y2="17" /></svg>
        </span>
        <div class="stat-body">
          <span class="stat-value">{{ overview?.messages ?? "—" }}</span>
          <span class="stat-label">消息</span>
        </div>
      </div>
      <!-- 成员卡：点击直达设置 tab 成员管理（头部骨架移走后的落位） -->
      <button class="stat-card stat-card-click" title="管理项目成员" @click="gotoMembers">
        <span class="stat-icon">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2" /><circle cx="9" cy="7" r="4" /><path d="M23 21v-2a4 4 0 0 0-3-3.87" /><path d="M16 3.13a4 4 0 0 1 0 7.75" /></svg>
        </span>
        <div class="stat-body">
          <span class="stat-value">{{ memberCount }}</span>
          <span class="stat-label">成员</span>
        </div>
      </button>

      <!-- 任务状态宽卡：五桶堆叠条 + 图例（零任务优雅空态） -->
      <div class="stat-card task-mix">
        <div v-if="taskTotal > 0" class="mix-body">
          <div class="mix-bar" role="img" :aria-label="`任务状态：进行中 ${buckets.running}，已完成 ${buckets.done}`">
            <span
              v-for="k in bucketKeys"
              v-show="buckets[k] > 0"
              :key="k"
              class="mix-seg"
              :class="k"
              :style="{ flexGrow: buckets[k] }"
            />
          </div>
          <div class="mix-legend">
            <span v-for="k in bucketKeys" v-show="buckets[k] > 0" :key="k" class="mix-item">
              <i class="dot" :class="k" />{{ TASK_STATUS_LABELS[k] }} {{ buckets[k] }}
            </span>
          </div>
        </div>
        <div v-else class="mix-empty">
          <span class="mix-empty-text">暂无委派任务</span>
          <span class="mix-empty-hint">在项目会话里让 agent 委派专家开工</span>
        </div>
      </div>
    </div>

    <!-- ===== 会话入口：最近会话（完整列表在侧栏） ===== -->
    <section class="conv-section">
      <div class="section-head">
        <h3 class="section-title">会话</h3>
        <span v-if="lastActivityLabel" class="section-meta">最近活动 {{ lastActivityLabel }}</span>
        <button class="view-all" @click="viewAll">在侧栏查看全部</button>
      </div>

      <div v-if="recentConvs.length" class="conv-list">
        <button v-for="c in recentConvs" :key="c.id" class="conv-row" @click="openConv(c)">
          <span class="row-agent">{{ agentName(c.agent_id) }}</span>
          <span class="row-title">{{ c.title || "新对话" }}</span>
          <span v-if="chat.streamingConvIds.has(c.id)" class="row-streaming">
            <span class="stream-bars"><span class="bar" /><span class="bar" /><span class="bar" /></span>生成中
          </span>
          <span v-else class="row-time">{{ ago(c.updated_at) }}</span>
        </button>
      </div>
      <div v-else class="conv-empty">
        <span>项目内还没有会话</span>
        <span class="conv-empty-hint">在侧栏选中本项目空间后点「新建对话」即可开始</span>
      </div>
    </section>
    <!-- 任务台账区（v1 表格已撤）：重设计中——先空着，数据入口在统计卡
         分桶与「项目轨迹」tab（全量事件流） -->
  </div>
</template>

<style scoped>
.overview {
  flex: 1; min-height: 0;
  display: flex; flex-direction: column; gap: 18px;
  overflow-y: auto;
  max-width: 960px;
  width: 100%;
  margin: 0 auto;
  scrollbar-width: none;
}
.overview::-webkit-scrollbar { display: none; }

/* ===== 统计带 ===== */
.stat-band {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr)) minmax(0, 1.7fr);
  gap: 10px;
}
.stat-card {
  display: flex; align-items: center; gap: 12px;
  padding: 14px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast) var(--ip-ease-out);
}
.stat-card-click {
  cursor: pointer; font-family: inherit;
  text-align: left;
}
.stat-card-click:hover {
  border-color: var(--ip-primary-400);
  box-shadow: var(--ip-shadow-sm);
}
.stat-icon {
  display: flex; align-items: center; justify-content: center;
  width: 32px; height: 32px; flex-shrink: 0;
  border-radius: var(--ip-radius-md);
  background-color: var(--ip-color-primary-tint-bg);
  color: var(--ip-color-primary-tint-text);
}
.stat-body { display: flex; flex-direction: column; gap: 2px; min-width: 0; }
.stat-value {
  font-size: 22px; line-height: 1.1;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  font-variant-numeric: tabular-nums;
}
.stat-label { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* 任务状态宽卡 */
.task-mix { padding: 12px 16px; }
.mix-body { display: flex; flex-direction: column; gap: 8px; align-self: stretch; flex: 1; justify-content: center; }
.mix-bar {
  display: flex; gap: 2px;
  height: 8px; border-radius: var(--ip-radius-full);
  overflow: hidden;
}
.mix-seg { height: 100%; min-width: 4px; }
.mix-seg.running { background: var(--ip-primary-500); }
.mix-seg.done { background: var(--ip-success-base); }
.mix-seg.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.mix-seg.interrupted { background: var(--ip-warning-base); }
.mix-seg.ended-other { background: var(--ip-color-text-tertiary); }

.mix-legend { display: flex; flex-wrap: wrap; gap: 4px 14px; }
.mix-item {
  display: inline-flex; align-items: center; gap: 5px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  font-variant-numeric: tabular-nums;
}
.dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
.dot.running { background: var(--ip-primary-500); animation: mix-pulse 1.2s ease-in-out infinite; }
.dot.done { background: var(--ip-success-base); }
.dot.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.dot.interrupted { background: var(--ip-warning-base); }
.dot.ended-other { background: var(--ip-color-text-tertiary); }
@keyframes mix-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

.mix-empty { display: flex; flex-direction: column; gap: 2px; align-self: center; }
.mix-empty-text { font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary); }
.mix-empty-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

/* ===== 会话入口 ===== */
.conv-section {
  display: flex; flex-direction: column; gap: 8px;
}
.section-head { display: flex; align-items: baseline; gap: 12px; }
.section-title {
  margin: 0;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.section-meta { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.view-all {
  margin-left: auto;
  border: none; background: none; cursor: pointer; font-family: inherit;
  font-size: var(--ip-text-caption-size); color: var(--ip-primary-600);
  padding: 2px 4px; border-radius: var(--ip-radius-sm);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.view-all:hover { background-color: var(--ip-color-primary-soft-bg, transparent); }

.conv-list {
  display: flex; flex-direction: column; gap: 4px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  padding: 6px;
}
.conv-row {
  display: flex; align-items: center; gap: 12px;
  padding: 9px 12px;
  border: none; border-radius: var(--ip-radius-md);
  background: none; cursor: pointer; font-family: inherit; text-align: left;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.conv-row:hover { background-color: var(--ip-color-bg-tertiary); }
.row-agent {
  flex-shrink: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-600);
  font-weight: var(--ip-font-weight-medium);
}
.row-title {
  flex: 1; min-width: 0;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.row-time { flex-shrink: 0; font-size: 11px; color: var(--ip-color-text-disabled); font-variant-numeric: tabular-nums; }
.row-streaming {
  flex-shrink: 0;
  display: inline-flex; align-items: center; gap: 4px;
  font-size: 11px; color: var(--ip-color-primary-tint-text);
}
.stream-bars { display: inline-flex; align-items: center; gap: 2px; height: 11px; }
.stream-bars .bar {
  width: 2px; height: 100%; border-radius: 1px;
  background: var(--ip-primary-500);
  animation: row-bar-bounce 0.9s ease-in-out infinite;
}
.stream-bars .bar:nth-child(2) { animation-delay: 0.15s; }
.stream-bars .bar:nth-child(3) { animation-delay: 0.3s; }
@keyframes row-bar-bounce {
  0%, 100% { transform: scaleY(0.35); opacity: 0.55; }
  50% { transform: scaleY(1); opacity: 1; }
}

.conv-empty {
  display: flex; flex-direction: column; gap: 4px;
  padding: 22px 16px;
  align-items: center; justify-content: center;
  background-color: var(--ip-color-bg-secondary);
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
}
.conv-empty-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
</style>

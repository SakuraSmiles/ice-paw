<script setup lang="ts">
// ProjectOverview.vue — 项目详情「概览」tab（2026-08-18 二轮重设计）：
//   纯统计带——4 数字卡（会话/委派任务/消息/成员）+ 任务状态宽卡（五桶
//   堆叠条 + 图例 + 最近活动）。纯排版（无图标块/无 hover 装饰，数字本体
//   即层级），Linear insights 式克制。
// 会话入口撤下（用户拍板：完整列表在侧栏，不重复造）；任务台账 v1 表格
// 已撤（重设计中，数据入口在统计分桶 + 轨迹 tab）。
// live：useProjectTasks 事件驱动（delegation-started / turn_ended 去抖）；
// keep-alive 离开期间错过的事件由 onActivated 补拉兜底。
import { computed, onActivated, onMounted, onUnmounted, ref, watch } from "vue";
import { useRoute, useRouter } from "vue-router";
import { bridge } from "../../api/bridge";
import { useProjectTasks } from "../../composables/useProjectTasks";
import { useChatStore } from "../../stores/chat";
import { useProjectStore } from "../../stores/project";
import { taskStatus, TASK_STATUS_LABELS } from "../../utils/taskStatus";
import { timeAgo } from "../../utils/time";
import type { ProjectOverview as OverviewData } from "../../types";

const route = useRoute();
const router = useRouter();
const chat = useChatStore();
const project = useProjectStore();

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

// 相对时间每分钟刷新（Sidebar nowTick 同款机制）
const nowTick = ref(Date.now());
let tick: ReturnType<typeof setInterval> | null = null;
onMounted(() => { tick = setInterval(() => (nowTick.value = Date.now()), 60000); });
onUnmounted(() => { if (tick) clearInterval(tick); });

function ago(iso: string): string {
  return timeAgo(iso, nowTick.value);
}

const lastActivityLabel = computed(() => {
  const at = overview.value?.last_activity_at;
  return at ? ago(at) : "";
});

/** 成员卡点击 → 设置 tab（成员管理所在） */
function gotoMembers() {
  router.push(`/projects/${projectId.value}/settings`);
}

const memberCount = computed(() => project.getById(projectId.value)?.agents?.length ?? 0);

// ---- 任务状态宽卡：五桶（running overlay 后的实时视角） ----
const bucketKeys = ["running", "done", "failed", "interrupted", "ended-other"] as const;
const buckets = computed(() => {
  const b = { running: 0, done: 0, failed: 0, interrupted: 0, "ended-other": 0 };
  for (const t of tasks.value) b[taskStatus(t, chat.streamingConvIds)] += 1;
  return b;
});
const taskTotal = computed(() => tasks.value.length);
</script>

<template>
  <div class="overview">
    <!-- ===== 统计带：4 数字卡 + 任务状态宽卡 ===== -->
    <div class="stat-band">
      <div class="stat-card">
        <span class="stat-value">{{ overview?.chat_conversations ?? "—" }}</span>
        <span class="stat-label">会话</span>
      </div>
      <div class="stat-card">
        <span class="stat-value">{{ overview?.delegation_conversations ?? "—" }}</span>
        <span class="stat-label">委派任务</span>
      </div>
      <div class="stat-card">
        <span class="stat-value">{{ overview?.messages ?? "—" }}</span>
        <span class="stat-label">消息</span>
      </div>
      <!-- 成员卡：点击直达设置 tab 成员管理 -->
      <button class="stat-card stat-card-click" title="管理项目成员" @click="gotoMembers">
        <span class="stat-value">{{ memberCount }}</span>
        <span class="stat-label">
          成员
          <svg class="label-chevron" width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="9 18 15 12 9 6" /></svg>
        </span>
      </button>

      <!-- 任务状态宽卡：五桶堆叠条 + 图例 + 项目节奏 -->
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
        <div v-else class="mix-empty">暂无委派任务</div>
        <div class="mix-meta">
          <span>共 {{ taskTotal }} 个任务</span>
          <span v-if="lastActivityLabel">最近活动 {{ lastActivityLabel }}</span>
        </div>
      </div>
    </div>
    <!-- 会话入口（用户拍板撤下：完整列表在侧栏）与任务台账区（重设计中）
         先空着，只留统计带 -->
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

/* ===== 统计带：纯排版（数字本体即层级，无图标块/无装饰） ===== */
.stat-band {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr)) minmax(0, 1.9fr);
  gap: 12px;
  align-items: stretch;
}
.stat-card {
  display: flex; flex-direction: column; justify-content: center;
  gap: 6px;
  padding: 18px 20px;
  background-color: var(--ip-color-bg-secondary);
  border-radius: var(--ip-radius-lg);
  font-family: inherit; text-align: left;
}
.stat-card-click {
  border: none; cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.stat-card-click:hover { background-color: var(--ip-color-bg-tertiary); }
.stat-card-click:hover .stat-value { color: var(--ip-primary-600); }

.stat-value {
  font-size: 28px; line-height: 1.1;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  font-variant-numeric: tabular-nums;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}
.stat-label {
  display: inline-flex; align-items: center; gap: 3px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
}
.label-chevron { opacity: 0; margin-left: -2px; transition: opacity var(--ip-duration-fast) var(--ip-ease-out); }
.stat-card-click:hover .label-chevron { opacity: 0.8; }

/* 任务状态宽卡 */
.task-mix { padding: 16px 20px; gap: 10px; }
.mix-body { display: flex; flex-direction: column; gap: 10px; flex: 1; justify-content: center; }
.mix-bar {
  display: flex; gap: 2px;
  height: 6px; border-radius: var(--ip-radius-full);
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

.mix-empty {
  flex: 1; display: flex; align-items: center;
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
}
.mix-meta {
  display: flex; justify-content: space-between; gap: 12px;
  padding-top: 8px;
  border-top: 1px solid var(--ip-color-border-default);
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}
</style>

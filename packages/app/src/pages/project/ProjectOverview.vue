<script setup lang="ts">
// ProjectOverview.vue — 项目详情「概览 · 任务台账」tab（MA-2）：
// 统计卡（会话/任务/消息/最近活跃 + 任务分桶）+ TaskLedger 台账。
// live：useProjectTasks 事件驱动（delegation-started / turn_ended 去抖 300ms）；
// keep-alive 离开期间错过的事件由 onActivated 补拉兜底。
import { computed, onActivated, ref, watch } from "vue";
import { useRoute } from "vue-router";
import { bridge } from "../../api/bridge";
import TaskLedger from "../../components/project/TaskLedger.vue";
import { useProjectTasks } from "../../composables/useProjectTasks";
import { taskStatus } from "../../utils/taskStatus";
import { useChatStore } from "../../stores/chat";
import { formatTime } from "../../utils/time";
import type { ProjectOverview } from "../../types";

const route = useRoute();
const chat = useChatStore();

const projectId = computed(() => String(route.params.id ?? ""));
const overview = ref<ProjectOverview | null>(null);
const overviewError = ref("");

const { tasks, loading: tasksLoading, load: loadTasks, refresh } = useProjectTasks(projectId);

async function loadOverview() {
  overviewError.value = "";
  try {
    overview.value = await bridge.projects.getOverview(projectId.value);
  } catch (e) {
    overviewError.value = e instanceof Error ? e.message : "加载项目概览失败";
  }
}

/** 任务分桶（流式 overlay 后的实时视角，统计卡随 running 翻转） */
const buckets = computed(() => {
  const b = { running: 0, done: 0, failed: 0, interrupted: 0, "ended-other": 0 };
  for (const t of tasks.value) b[taskStatus(t, chat.streamingConvIds)] += 1;
  return b;
});

watch(projectId, () => {
  void loadOverview();
  void loadTasks();
}, { immediate: true });

// keep-alive 回到本 tab：补拉（离开期间错过 delegation-started/turn_ended）
onActivated(() => {
  void loadOverview();
  refresh();
});

const lastActivity = computed(() =>
  overview.value?.last_activity_at ? formatTime(overview.value.last_activity_at) : "—",
);
</script>

<template>
  <div class="overview">
    <!-- 统计卡 -->
    <div class="stat-grid">
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
      <div class="stat-card">
        <span class="stat-value stat-time">{{ lastActivity }}</span>
        <span class="stat-label">最近活跃</span>
      </div>

      <div class="stat-card wide">
        <div class="bucket-row">
          <span class="bucket running"><i class="dot running" />进行中 {{ buckets.running }}</span>
          <span class="bucket done"><i class="dot done" />已完成 {{ buckets.done }}</span>
          <span class="bucket failed"><i class="dot failed" />未成功 {{ buckets.failed }}</span>
          <span class="bucket interrupted"><i class="dot interrupted" />中断 {{ buckets.interrupted }}</span>
          <span class="bucket other"><i class="dot other" />补录 {{ buckets["ended-other"] }}</span>
        </div>
      </div>
    </div>
    <div v-if="overviewError" class="load-error">{{ overviewError }}</div>

    <!-- 任务台账 -->
    <section class="ledger-section">
      <div class="section-head">
        <h3 class="section-title">任务台账</h3>
        <span v-if="tasksLoading" class="section-hint">加载中…</span>
        <span v-else class="section-hint">任务 ≡ 委派子会话，实时入账 · 点击行查看轨迹</span>
      </div>
      <TaskLedger :tasks="tasks" />
    </section>
  </div>
</template>

<style scoped>
.overview {
  flex: 1; min-height: 0;
  display: flex; flex-direction: column; gap: 18px;
  overflow-y: auto;
}

.stat-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr)) minmax(0, 1.6fr);
  gap: 10px;
}
.stat-card {
  display: flex; flex-direction: column; gap: 4px;
  padding: 14px 16px;
  background-color: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
}
.stat-value {
  font-size: 22px; font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  font-variant-numeric: tabular-nums;
}
.stat-value.stat-time { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-medium); }
.stat-label { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.bucket-row { display: flex; flex-wrap: wrap; gap: 6px 16px; }
.bucket {
  display: inline-flex; align-items: center; gap: 6px;
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary);
  font-variant-numeric: tabular-nums;
}
.dot { width: 7px; height: 7px; border-radius: 50%; }
.dot.running { background: var(--ip-primary-500); animation: dot-pulse 1.2s ease-in-out infinite; }
.dot.done { background: var(--ip-success-base); }
.dot.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.dot.interrupted { background: var(--ip-warning-base); }
.dot.other { background: var(--ip-color-text-tertiary); }
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

.load-error { font-size: var(--ip-text-caption-size); color: var(--ip-danger-text); }

.ledger-section {
  display: flex; flex-direction: column; gap: 8px;
  min-height: 0;
}
.section-head { display: flex; align-items: baseline; gap: 10px; }
.section-title {
  margin: 0;
  font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}
.section-hint { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
</style>

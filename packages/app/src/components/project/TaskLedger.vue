<script setup lang="ts">
// TaskLedger.vue — 项目任务台账（MA-2）：项目内全部委派任务的状态视图。
// 数据源 useProjectTasks（父级传入静态行），本组件负责：
// - 终态推导（utils/taskStatus + streamingConvIds overlay，running 恒优先）
// - 排序：running 置顶，其余 updated_at 倒序（TaskPanel 先例）
// - 行点击 → 首页 + 落到该子会话轨迹 tab（MA-1 一次性标志通道）
import { computed } from "vue";
import { useRouter } from "vue-router";
import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { taskStatus, TASK_STATUS_LABELS } from "../../utils/taskStatus";
import { formatTime, parseDbTime } from "../../utils/time";
import type { ProjectTask } from "../../types";

const props = defineProps<{ tasks: ProjectTask[] }>();

const router = useRouter();
const chat = useChatStore();
const agent = useAgentStore();

type Row = {
  task: ProjectTask;
  status: ReturnType<typeof taskStatus>;
  /** 发起者名（null ≡ 用户发起） */
  initiatorName: string;
  durationLabel: string;
};

const rows = computed<Row[]>(() => {
  const enriched = props.tasks.map((task) => {
    const status = taskStatus(task, chat.streamingConvIds);
    return {
      task,
      status,
      initiatorName: task.initiator_agent_id
        ? agent.getById(task.initiator_agent_id)?.name ?? "未知"
        : "用户",
      durationLabel: durationLabel(task, status),
    };
  });
  // running 置顶（脉冲在视线最上方），其余 updated_at 倒序
  const rank = { running: 0, done: 1, failed: 2, interrupted: 3, "ended-other": 4 } as const;
  return enriched.sort((a, b) => {
    const ra = rank[a.status];
    const rb = rank[b.status];
    if (ra !== rb) return ra - rb;
    return parseDbTime(b.task.updated_at).getTime() - parseDbTime(a.task.updated_at).getTime();
  });
});

/** 耗时：ended_at - started_at；未结束（running/interrupted）用当前时间补 */
function durationLabel(task: ProjectTask, status: string): string {
  const start = parseDbTime(task.started_at).getTime();
  const end =
    task.ended_at != null && status !== "running"
      ? parseDbTime(task.ended_at).getTime()
      : Date.now();
  const sec = Math.max(0, Math.round((end - start) / 1000));
  if (sec < 60) return `${sec}s`;
  if (sec < 3600) return `${Math.floor(sec / 60)}m${sec % 60}s`;
  const h = Math.floor(sec / 3600);
  return `${h}h${Math.floor((sec % 3600) / 60)}m`;
}

function openTask(task: ProjectTask) {
  router.push("/");
  chat.openConversationAtTrajectory(task.conv_id);
}
</script>

<template>
  <div class="ledger">
    <div v-if="rows.length === 0" class="ledger-empty">
      还没有委派任务——在项目会话里让 agent 委派专家开工，任务会实时入账。
    </div>
    <template v-else>
      <div class="ledger-head">
        <span class="col-state">状态</span>
        <span class="col-title">任务</span>
        <span class="col-agent">执行</span>
        <span class="col-agent">发起</span>
        <span class="col-num">耗时</span>
        <span class="col-num">轮数</span>
        <span class="col-time">最近活动</span>
      </div>
      <button
        v-for="r in rows"
        :key="r.task.conv_id"
        class="ledger-row"
        :title="`${TASK_STATUS_LABELS[r.status]} · 点击查看任务轨迹`"
        @click="openTask(r.task)"
      >
        <span class="col-state">
          <span class="state-dot" :class="r.status" />
          <span class="state-label" :class="r.status">{{ TASK_STATUS_LABELS[r.status] }}</span>
        </span>
        <span class="col-title">{{ r.task.title || "委派任务" }}</span>
        <span class="col-agent">{{ agent.getById(r.task.executor_agent_id)?.name ?? "未知" }}</span>
        <span class="col-agent">{{ r.initiatorName }}</span>
        <span class="col-num">{{ r.durationLabel }}</span>
        <span class="col-num">{{ r.task.rounds ?? "—" }}</span>
        <span class="col-time">{{ formatTime(r.task.updated_at) }}</span>
      </button>
    </template>
  </div>
</template>

<style scoped>
.ledger { display: flex; flex-direction: column; gap: 2px; }

.ledger-empty {
  padding: 20px 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.ledger-head, .ledger-row {
  display: grid;
  grid-template-columns: 92px minmax(0, 1fr) 90px 90px 64px 48px 88px;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: 6px 10px;
}
.ledger-head {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
  position: sticky; top: 0;
  background: var(--ip-color-bg-primary);
  z-index: 1;
}
.ledger-row {
  border: none; border-radius: var(--ip-radius-md);
  background: none; cursor: pointer;
  font-family: inherit; text-align: left;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ledger-row:hover { background-color: var(--ip-color-bg-tertiary); }

.col-title {
  min-width: 0;
  overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  color: var(--ip-color-text-primary);
}
.col-agent { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.col-num { font-variant-numeric: tabular-nums; text-align: right; }
.col-time { font-variant-numeric: tabular-nums; color: var(--ip-color-text-disabled); }

.col-state { display: flex; align-items: center; gap: 6px; }

/* 状态点五态：running 脉冲 / done 绿 / failed 红 / interrupted 琥珀 /
   ended-other 中性（历史补录诚实标注，非异常） */
.state-dot {
  width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0;
  background: var(--ip-color-text-tertiary);
}
.state-dot.running { background: var(--ip-primary-500); animation: dot-pulse 1.2s ease-in-out infinite; }
.state-dot.done { background: var(--ip-success-base); }
.state-dot.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.state-dot.interrupted { background: var(--ip-warning-base); }
@keyframes dot-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

.state-label { font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary); }
.state-label.running { color: var(--ip-primary-600); }
.state-label.done { color: var(--ip-success-text); }
.state-label.failed { color: var(--ip-danger-text); }
</style>

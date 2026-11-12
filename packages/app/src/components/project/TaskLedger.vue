<script setup lang="ts">
// TaskLedger.vue — 项目任务面板 v2（批 P2-2，person-first；接棒 v1 撤下后的重设计）。
//
// 设计拍板 2026-11-12（docs/channel-router-roadmap.md §7 P2-b + DSH 005-panel）：
// - 分区排序：**等你回应 > 执行中 > 失败受阻 > 已完成**——最需要用户注意力的
//   永远置顶；等回答 = 任务会话有未作答 ask_user（chat store live Map，进程内
//   语义——ask 的 oneshot 随进程存亡，无历史悬置态）。
// - 维度：人（执行者头像+名 · 发起者）/ 状态与轮次 / 时间（相对时+hover 绝对时）。
//   成本维度（每任务 Σ token）是 P2-3 接线，行结构已留位。
// - 消息流减负：进度全貌在面板，对话流只留汇报卡（P2-1b）——两级视角互补
//   （会话级 = TaskPanel 胶囊，项目级 = 本面板）。
// - 视觉基线 = v1（状态点 + 行布局），Phase 3 router 上线后长出任务图形态。
// 数据源 useProjectTasks（父级传入，live 管线在 composable）；点击行 →
// 落到该子会话轨迹 tab（openConversationAtTrajectory 一次性标志通道）。
import { computed } from "vue";
import { useChatStore } from "../../stores/chat";
import { useAgentStore } from "../../stores/agent";
import { buildBoard, SECTION_TITLES, type BoardSection } from "../../utils/taskBoard";
import { TASK_STATUS_LABELS } from "../../utils/taskStatus";
import { parseDbTime, timeAgo, formatDate, formatTime } from "../../utils/time";
import type { ProjectTask } from "../../types";
import EntityAvatar from "../common/EntityAvatar.vue";
import StatusGlyph from "../chat/StatusGlyph.vue";

const props = defineProps<{ tasks: ProjectTask[] }>();

const chat = useChatStore();
const agent = useAgentStore();

/** 等你回答的问题投影（convId → 摘要）——live Map 直投，无持久化语义 */
const waitingQuestions = computed<ReadonlyMap<string, string>>(() => {
  const m = new Map<string, string>();
  for (const [cid, entry] of chat.pendingAskRequests) {
    m.set(cid, entry.payload.question ?? "等待你的选择");
  }
  return m;
});

interface Row {
  key: string;
  section: BoardSection;
  statusLabel: string;
  executorName: string;
  executorImage: string | null;
  /** 发起者标注（null ≡ 用户发起，显示「用户」淡字） */
  initiatorLabel: string | null;
  title: string;
  rounds: number | null;
  waitingQuestion: string | null;
  timeLabel: string;
  timeTitle: string;
}

const rows = computed<Row[]>(() =>
  buildBoard(props.tasks, chat.streamingConvIds, waitingQuestions.value).map((r) => {
    const executor = agent.getById(r.task.executor_agent_id);
    const updated = parseDbTime(r.task.updated_at);
    return {
      key: r.task.conv_id,
      section: r.section,
      statusLabel: TASK_STATUS_LABELS[r.status],
      executorName: executor?.name ?? "未知执行者",
      executorImage: executor?.avatar ?? null,
      initiatorLabel: r.task.initiator_agent_id
        ? agent.getById(r.task.initiator_agent_id)?.name ?? "未知"
        : null,
      title: r.task.title || "任务",
      rounds: r.task.rounds,
      waitingQuestion: r.waitingQuestion,
      timeLabel: timeAgo(updated.toISOString()),
      timeTitle: `${formatDate(updated.toISOString())} ${formatTime(updated.toISOString())}`,
    };
  }),
);

const sections = computed(() =>
  (Object.keys(SECTION_TITLES) as BoardSection[])
    .map((s) => ({ key: s, title: SECTION_TITLES[s], rows: rows.value.filter((r) => r.section === s) }))
    .filter((s) => s.rows.length > 0),
);

const counts = computed(() => {
  const c: Record<BoardSection, number> = { waiting: 0, running: 0, failed: 0, done: 0 };
  for (const r of rows.value) c[r.section] += 1;
  return c;
});

function openTask(convId: string) {
  chat.openConversationAtTrajectory(convId);
}
</script>

<template>
  <div class="board">
    <div class="board-head">
      <span class="board-title">任务面板</span>
      <span class="board-meta">
        <template v-if="counts.waiting > 0"><b class="warn">{{ counts.waiting }} 个等你回应</b> · </template>
        进行中 {{ counts.running }} · 失败 {{ counts.failed }} · 已完成 {{ counts.done }}
      </span>
    </div>
    <div v-if="rows.length === 0" class="board-empty">
      暂无任务——在会话里委派任务（delegate_to_agent / start_delegate）或创建临时工作者后，这里会呈现全景。
    </div>
    <section v-for="s in sections" :key="s.key" class="board-section" :data-section="s.key">
      <div class="sec-head">
        <i class="sec-dot" :class="s.key" />
        <span class="sec-title">{{ s.title }}</span>
        <span class="sec-count">{{ s.rows.length }}</span>
      </div>
      <button
        v-for="r in s.rows"
        :key="r.key"
        class="task-row"
        :title="`${r.executorName} · ${r.statusLabel}${r.rounds != null ? ` · ${r.rounds} 轮` : ''} · ${r.timeTitle}`"
        @click="openTask(r.key)"
      >
        <EntityAvatar class="row-avatar" :name="r.executorName" :image="r.executorImage" size="sm" />
        <span class="row-main">
          <span class="row-title">{{ r.title }}</span>
          <span v-if="r.waitingQuestion" class="row-waiting" :title="r.waitingQuestion">
            等你回答：{{ r.waitingQuestion }}
          </span>
        </span>
        <span class="row-meta">
          <span class="row-executor">{{ r.executorName }}</span>
          <span v-if="r.initiatorLabel" class="row-initiator">← {{ r.initiatorLabel }} 发起</span>
          <StatusGlyph
            :status="s.key === 'waiting' ? 'pending' : s.key === 'running' ? 'running' : s.key === 'failed' ? 'error' : 'done'"
            :size="12"
            :label="s.key === 'running' ? undefined : r.statusLabel"
          />
          <span class="row-time" :title="r.timeTitle">{{ r.timeLabel }}</span>
        </span>
      </button>
    </section>
  </div>
</template>

<style scoped>
.board {
  display: flex; flex-direction: column; gap: var(--ip-spacing-3);
  background-color: var(--ip-color-bg-secondary);
  border-radius: var(--ip-radius-lg);
  padding: var(--ip-spacing-4) var(--ip-spacing-5);
}
.board-head { display: flex; align-items: baseline; gap: var(--ip-spacing-2); }
.board-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}
.board-meta {
  font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}
.board-meta .warn { color: var(--ip-warning-text); font-weight: var(--ip-font-weight-medium); }
.board-empty {
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-secondary);
  padding: var(--ip-spacing-2) 0;
}

.board-section { display: flex; flex-direction: column; gap: var(--ip-spacing-1); }
.sec-head { display: flex; align-items: center; gap: var(--ip-spacing-1_5); padding: var(--ip-spacing-1) 0; }
.sec-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }
.sec-dot.waiting { background: var(--ip-warning-base); animation: sec-pulse 1.2s ease-in-out infinite; }
.sec-dot.running { background: var(--ip-primary-500); animation: sec-pulse 1.2s ease-in-out infinite; }
.sec-dot.failed { background: var(--ip-danger-base, var(--ip-danger-text)); }
.sec-dot.done { background: var(--ip-success-base); }
@keyframes sec-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
.sec-title {
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}
.sec-count {
  font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary);
  font-variant-numeric: tabular-nums;
}

.task-row {
  display: flex; align-items: center; gap: var(--ip-spacing-2);
  border: none; background: none; cursor: pointer; text-align: left;
  padding: var(--ip-spacing-1_5) var(--ip-spacing-2);
  border-radius: var(--ip-radius-md);
  font-family: inherit;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.task-row:hover { background-color: var(--ip-color-bg-tertiary); }
.row-avatar { flex-shrink: 0; }
.row-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
.row-title {
  font-size: var(--ip-text-body-sm-size); color: var(--ip-color-text-primary);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.row-waiting {
  font-size: var(--ip-text-micro-size); color: var(--ip-warning-text);
  white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
}
.row-meta {
  display: flex; align-items: center; gap: var(--ip-spacing-1_5);
  flex-shrink: 0; font-size: var(--ip-text-micro-size); color: var(--ip-color-text-tertiary);
}
.row-executor { color: var(--ip-color-text-secondary); max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row-initiator { max-width: 120px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.row-time { font-variant-numeric: tabular-nums; min-width: 3em; text-align: right; }
</style>

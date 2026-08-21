<script setup lang="ts">
// DelegationCard.vue — 父会话里 delegate_to_agent 的委派卡片（MA-1，v1 最小要素）
//
// 取代该工具调用的通用工具行渲染：目标 agent / 状态 / 跳转子会话轨迹。
// 形态不锁设计（评审拍板：实现期用户看着效果边看边调），本轮只保骨架：
// - 进行中：任务摘要 + 呼吸点；chat:delegation-started 推送后子会话创建即
//   可达（运行中也可跳——childConvId 由 ChatMessages 取数层从 store 补齐）
// - 完成/失败：专家名 + 轮数 + finish_reason 徽章 + 「打开任务」入口
// 展开看原始参数/结果走原工具行（本卡片不重复承载，减少双份维护）。
import { computed } from "vue";
import { useAgentStore } from "../../stores/agent";
import EntityAvatar from "../common/EntityAvatar.vue";

const props = defineProps<{
  /** 目标 agent 显示名（参数里的 agent_id 原样兜底） */
  agentName: string;
  /** 目标 agent id（可解析时用于头像；流式参数未到齐为 null → 走名字渐变兜底） */
  agentId?: string | null;
  /** 委派任务文本（参数 task，截断展示） */
  task: string;
  /** running=进行中（无结果）；done=正常回传；error=Err 回传 */
  status: "running" | "done" | "error";
  /** 子会话 id（完成后才有；运行中为 null → 跳转钮隐藏） */
  childConvId?: string | null;
  /** 专家跑了多少轮（done 时展示） */
  rounds?: number | null;
  /** 终止原因词表（stop/cancelled/budget_exceeded/…） */
  finishReason?: string | null;
}>();

const agent = useAgentStore();
/** 头像取数：agentId 命中 store 才有 image，否则纯名字兜底。 */
const targetAgent = computed(() => (props.agentId ? agent.getById(props.agentId) : undefined));

const emit = defineEmits<{ (e: "open-child", childId: string): void }>();

function openChild() {
  if (props.childConvId) emit("open-child", props.childConvId);
}

const STATUS_TEXT: Record<string, string> = {
  running: "执行中",
  done: "已完成",
  error: "失败",
};

const FINISH_LABEL: Record<string, string> = {
  stop: "正常完成",
  cancelled: "已取消",
  abort: "中止",
  budget_exceeded: "预算触顶",
  stuck: "疑似卡住",
};
</script>

<template>
  <div class="dlg-card" :data-status="status">
    <div class="dlg-head">
      <EntityAvatar
        class="dlg-avatar"
        :name="targetAgent?.name ?? agentName"
        :image="targetAgent?.avatar ?? null"
        size="sm"
      />
      <span class="dlg-title">委派给 {{ agentName }}</span>
      <span class="dlg-status">
        <span class="dlg-dot" :data-status="status" />
        {{ STATUS_TEXT[status] }}
      </span>
    </div>
    <div class="dlg-task" :title="task">{{ task }}</div>
    <div class="dlg-foot">
      <span v-if="status !== 'running'" class="dlg-meta">
        <template v-if="status === 'done'">
          {{ finishReason === 'stop' ? FINISH_LABEL.stop : (finishReason ? `${FINISH_LABEL[finishReason] ?? finishReason}（${finishReason}）` : '') }}
          <template v-if="rounds != null"> · {{ rounds }} 轮</template>
        </template>
        <template v-else>调用失败，详见展开的原始结果</template>
      </span>
      <button
        v-if="childConvId"
        class="dlg-open"
        title="打开任务详情（子会话，默认轨迹视图）"
        @click.stop="openChild"
      >
        打开任务
      </button>
    </div>
  </div>
</template>

<style scoped>
/* 委派卡片与工具行同族视觉（tools-strip 语境），但作为「会话级事件」用左侧
   竖线强调边；底色遵循 tint 令牌约定（soft-bg），不直接用 primary-50/100。 */
.dlg-card {
  border: 1px solid var(--ip-color-border-default);
  border-left: 3px solid var(--ip-primary-400);
  border-radius: var(--ip-radius-md);
  background: var(--ip-primary-soft-bg, var(--ip-color-bg-secondary));
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 5px;
  max-width: 560px;
}
.dlg-card[data-status="error"] { border-left-color: var(--ip-danger-base); }
.dlg-card[data-status="running"] { border-left-color: var(--ip-warning-base); }

.dlg-head { display: flex; align-items: center; gap: 6px; }
/* 目标 agent 头像（sm=20px，EntityAvatar 三级链；取代旧交换箭头图标） */
.dlg-avatar { flex-shrink: 0; }
.dlg-title { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.dlg-status { margin-left: auto; display: flex; align-items: center; gap: 5px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }

.dlg-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; }
.dlg-dot[data-status="running"] { background: var(--ip-warning-base); animation: dlg-pulse 1.2s ease-in-out infinite; }
.dlg-dot[data-status="done"] { background: var(--ip-success-base); }
.dlg-dot[data-status="error"] { background: var(--ip-danger-base); }
@keyframes dlg-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

.dlg-task {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-secondary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.dlg-foot { display: flex; align-items: center; gap: var(--ip-spacing-2); min-height: 22px; }
.dlg-meta { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.dlg-open {
  margin-left: auto;
  border: none;
  background: none;
  color: var(--ip-primary-600);
  font-size: var(--ip-text-caption-size);
  cursor: pointer;
  padding: 2px 6px;
  border-radius: var(--ip-radius-sm);
}
.dlg-open:hover { background: var(--ip-primary-soft-bg, rgba(0, 0, 0, 0.05)); }
</style>

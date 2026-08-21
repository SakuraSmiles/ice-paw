<script setup lang="ts">
// PlanCard.vue — update_plan 工具调用的计划卡片（C5，v1）
//
// 与 DelegationCard 同族：取代该工具调用的通用工具行渲染。展示该次调用
// 声明的**全量快照**（后端语义：每次调用整体覆写，非增量打勾）。
// - 条目状态：○ pending / ● in_progress（呼吸）/ ✓ done（划线弱化）
// - task_conversation_id 存在的条目可跳对应任务（委派子会话，引用边）
// - 「N/M」进度 = done 数 / 总数；M=0（清空计划）时后端返回空清单，
//   此卡片仍渲染但显示「已清空」（诚实于历史调用记录）
// 展开看原始参数/结果走原工具行（本卡片不重复承载）。
import type { PlanItem } from "../../types";

const props = defineProps<{
  /** 该次 update_plan 调用声明的计划快照（整体覆写） */
  items: PlanItem[];
}>();

const emit = defineEmits<{ (e: "open-task", conversationId: string): void }>();

const doneCount = () => props.items.filter((it) => it.status === "done").length;

function openTask(it: PlanItem) {
  if (it.task_conversation_id) emit("open-task", it.task_conversation_id);
}
</script>

<template>
  <div class="plan-card">
    <div class="plan-head">
      <svg class="plan-icon" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M9 11l3 3L22 4"/><path d="M21 12v7a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h11"/></svg>
      <span class="plan-title">计划</span>
      <span class="plan-progress">{{ doneCount() }}/{{ items.length }}</span>
    </div>
    <div v-if="items.length === 0" class="plan-empty">已清空计划</div>
    <ul v-else class="plan-list">
      <li
        v-for="(it, i) in items"
        :key="i"
        :class="['plan-item', it.task_conversation_id ? 'plan-item-link' : '']"
        :title="it.task_conversation_id ? '此步骤挂有委派任务，点击打开' : it.text"
        @click="openTask(it)"
      >
        <span :class="['plan-mark', `plan-mark-${it.status}`]" />
        <span class="plan-text">{{ it.text }}</span>
        <svg v-if="it.task_conversation_id" class="plan-arrow" width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M5 12h14M12 5l7 7-7 7"/></svg>
      </li>
    </ul>
  </div>
</template>

<style scoped>
/* 计划卡片与 DelegationCard 同族（tools-strip 语境 + 左侧竖线强调边），
   但语义是「意图文档」而非「执行事件」——左边线用中性 primary。 */
.plan-card {
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
.plan-head { display: flex; align-items: center; gap: 6px; }
.plan-icon { color: var(--ip-primary-500); flex-shrink: 0; }
.plan-title { font-size: var(--ip-text-body-sm-size); font-weight: var(--ip-font-weight-semibold); color: var(--ip-color-text-primary); }
.plan-progress { margin-left: auto; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); font-variant-numeric: tabular-nums; }

.plan-empty { font-size: var(--ip-text-caption-size); color: var(--ip-color-text-tertiary); }
.plan-list { list-style: none; margin: 0; padding: 0; display: flex; flex-direction: column; gap: 3px; }
.plan-item { display: flex; align-items: center; gap: 7px; font-size: var(--ip-text-caption-size); color: var(--ip-color-text-secondary); }
.plan-item-link { cursor: pointer; border-radius: var(--ip-radius-sm); padding: 1px 4px; margin: 0 -4px; }
.plan-item-link:hover { background: var(--ip-color-bg-tertiary, rgba(0, 0, 0, 0.05)); }

.plan-mark { width: 9px; height: 9px; flex-shrink: 0; border-radius: 50%; border: 1.5px solid var(--ip-color-text-tertiary); }
.plan-mark-pending { border-color: var(--ip-color-text-tertiary); }
.plan-mark-in_progress { border-color: var(--ip-warning-base); background: var(--ip-warning-base); animation: plan-pulse 1.2s ease-in-out infinite; }
.plan-mark-done { border-color: var(--ip-success-base); background: var(--ip-success-base); }
@keyframes plan-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }

.plan-item .plan-mark-done + .plan-text { text-decoration: line-through; color: var(--ip-color-text-tertiary); }
.plan-text { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
.plan-arrow { color: var(--ip-primary-500); flex-shrink: 0; }
</style>

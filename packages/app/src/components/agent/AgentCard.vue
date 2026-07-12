<script setup lang="ts">
// Agent 卡片组件
//
// 职责：
//   - 展示单个 Agent 的摘要信息：名称、Provider、Model、创建时间
//   - 提供「编辑」和「删除」操作按钮
//
// props:
//   - agent: Agent 实体
//
// emits:
//   - edit:   点击编辑按钮时触发
//   - delete: 点击删除按钮时触发

import type { Agent } from "../../types";

const props = defineProps<{
  agent: Agent;
}>();

const emit = defineEmits<{
  edit: [agent: Agent];
  delete: [agent: Agent];
}>();

/** 格式化日期为友好文本 */
function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    const y = d.getFullYear();
    const m = String(d.getMonth() + 1).padStart(2, "0");
    const day = String(d.getDate()).padStart(2, "0");
    const h = String(d.getHours()).padStart(2, "0");
    const min = String(d.getMinutes()).padStart(2, "0");
    return `${y}-${m}-${day} ${h}:${min}`;
  } catch {
    return iso;
  }
}
</script>

<template>
  <div class="agent-card">
    <div class="agent-card-body">
      <div class="agent-name">{{ props.agent.name }}</div>
      <div class="agent-meta">
        <span class="agent-tag">{{ props.agent.provider }}</span>
        <span class="agent-tag">{{ props.agent.model }}</span>
      </div>
      <div class="agent-time">{{ formatDate(props.agent.created_at) }}</div>
    </div>
    <div class="agent-card-actions">
      <button class="btn-action btn-edit" title="编辑" @click="emit('edit', props.agent)">
        <!-- 铅笔图标用纯文字替代 -->
        <span>编辑</span>
      </button>
      <button class="btn-action btn-delete" title="删除" @click="emit('delete', props.agent)">
        <span>删除</span>
      </button>
    </div>
  </div>
</template>

<style scoped>
.agent-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 18px;
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  transition: box-shadow var(--ip-duration-fast) var(--ip-ease-out), border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.agent-card:hover {
  box-shadow: var(--ip-shadow-sm);
  border-color: var(--ip-color-border-strong);
}

.agent-card-body {
  flex: 1;
  min-width: 0;
}

.agent-name {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.agent-meta {
  display: flex;
  gap: var(--ip-spacing-2);
  margin-top: 6px;
}

.agent-tag {
  display: inline-block;
  padding: 2px var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-gray-600);
}

.agent-time {
  margin-top: var(--ip-spacing-1);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.agent-card-actions {
  display: flex;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
}

.btn-action {
  padding: var(--ip-spacing-1) var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-color-bg-primary);
  color: var(--ip-gray-600);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out), border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.btn-action:hover {
  background: var(--ip-color-bg-tertiary);
}

.btn-delete {
  color: var(--ip-danger-base);
  border-color: var(--ip-danger-border);
}
.btn-delete:hover {
  background: var(--ip-danger-bg);
}
</style>
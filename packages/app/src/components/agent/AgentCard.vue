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
  background: var(--card-bg, #ffffff);
  border: 1px solid var(--card-border, #e0e0e0);
  border-radius: 8px;
  transition: box-shadow 120ms ease, border-color 120ms ease;
}
.agent-card:hover {
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
  border-color: var(--card-border-hover, #c0c0c0);
}

.agent-card-body {
  flex: 1;
  min-width: 0;
}

.agent-name {
  font-size: 15px;
  font-weight: 600;
  color: var(--text-primary, #1a1a1a);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.agent-meta {
  display: flex;
  gap: 8px;
  margin-top: 6px;
}

.agent-tag {
  display: inline-block;
  padding: 2px 8px;
  font-size: 12px;
  border-radius: 4px;
  background: var(--tag-bg, #f0f0f0);
  color: var(--tag-fg, #555);
}

.agent-time {
  margin-top: 4px;
  font-size: 12px;
  color: var(--text-secondary, #888);
}

.agent-card-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.btn-action {
  padding: 4px 12px;
  font-size: 13px;
  border: 1px solid var(--btn-border, #d0d0d0);
  border-radius: 4px;
  background: var(--btn-bg, #fafafa);
  color: var(--text-secondary, #555);
  cursor: pointer;
  transition: background 100ms ease, border-color 100ms ease;
}
.btn-action:hover {
  background: var(--btn-bg-hover, #f0f0f0);
}

.btn-delete {
  color: var(--danger-fg, #d93025);
  border-color: var(--danger-border, #f5c6cb);
}
.btn-delete:hover {
  background: var(--danger-bg-hover, #fde8e8);
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .agent-card {
    --card-bg: #2a2a3a;
    --card-border: #3a3a4a;
    --card-border-hover: #5a5a6a;
  }
  .agent-name {
    --text-primary: #f0f0f0;
  }
  .agent-tag {
    --tag-bg: #3a3a4a;
    --tag-fg: #bbb;
  }
  .agent-time {
    --text-secondary: #888;
  }
  .btn-action {
    --btn-bg: #3a3a4a;
    --btn-border: #4a4a5a;
    --btn-bg-hover: #4a4a5a;
    --text-secondary: #ccc;
  }
  .btn-delete {
    --danger-fg: #ff6b6b;
    --danger-border: #5a2a2a;
  }
  .btn-delete:hover {
    --danger-bg-hover: #3a2020;
  }
}
</style>
<script setup lang="ts">
// Agent 卡片组件
//
// 职责：
//   - 展示单个 Agent 的摘要信息：头像（字母缩写/Lucide 图标）+ 名称 + 描述 + provider/model 标签 + 时间
//   - 提供「编辑」和「删除」操作按钮（@ice-paw/ui Button + lucide 图标）
//
// 布局（方案 v2 4.3）：
//   [头像] [名称 + 描述 + provider/model 标签] ........ [Edit] [Delete]
//
// props:
//   - agent: Agent 实体
//
// emits:
//   - edit:   点击编辑按钮时触发
//   - delete: 点击删除按钮时触发

import { computed } from "vue";
import { Button } from "@ice-paw/ui";
import { Pencil, Trash2 } from "lucide-vue-next";
import type { Agent } from "../../types";
import { useAgentMeta } from "../../composables/useAgentMeta";
import type { AgentMeta } from "../../composables/useAgentMeta";
import AgentAvatar from "../common/AgentAvatar.vue";

const props = defineProps<{
  agent: Agent;
}>();

const emit = defineEmits<{
  edit: [agent: Agent];
  delete: [agent: Agent];
}>();

const agentMeta = useAgentMeta();

/** Agent 完整元数据（含 icon） */
const meta = computed<AgentMeta | null>(() => agentMeta.getFullMeta(props.agent));

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
    <!-- 头像 -->
    <AgentAvatar v-if="meta" :meta="meta" :size="48" />
    <div v-else class="agent-avatar-placeholder" :style="{ width: '48px', height: '48px' }"></div>

    <!-- 信息区 -->
    <div class="agent-card-body">
      <div class="agent-name">{{ props.agent.name }}</div>
      <div v-if="meta?.description" class="agent-desc">{{ meta.description }}</div>
      <div class="agent-meta">
        <span class="agent-tag">{{ props.agent.provider }}</span>
        <span class="agent-tag">{{ props.agent.model }}</span>
      </div>
      <div class="agent-time">{{ formatDate(props.agent.created_at) }}</div>
    </div>

    <!-- 操作按钮 -->
    <div class="agent-card-actions">
      <Button
        variant="secondary"
        size="sm"
        :title="`编辑 ${props.agent.name}`"
        :aria-label="`编辑 ${props.agent.name}`"
        @click="emit('edit', props.agent)"
      >
        <template #icon-left>
          <Pencil :size="14" aria-hidden="true" />
        </template>
        编辑
      </Button>
      <Button
        variant="ghost"
        size="sm"
        :title="`删除 ${props.agent.name}`"
        :aria-label="`删除 ${props.agent.name}`"
        @click="emit('delete', props.agent)"
      >
        <template #icon-left>
          <Trash2 :size="14" aria-hidden="true" />
        </template>
        删除
      </Button>
    </div>
  </div>
</template>

<style scoped>
.agent-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-3);
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-xs);
  transition:
    box-shadow var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out);
}
.agent-card:hover {
  box-shadow: var(--ip-shadow-sm);
  border-color: var(--ip-color-border-strong);
}

.agent-card-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.agent-name {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.agent-desc {
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-tertiary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.agent-meta {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ip-spacing-2);
}

.agent-tag {
  display: inline-block;
  padding: 2px var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-relaxed);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-gray-600);
}

.agent-time {
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-tertiary);
}

.agent-card-actions {
  display: flex;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
}
</style>

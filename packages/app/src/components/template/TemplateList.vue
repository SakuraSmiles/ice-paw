<script setup lang="ts">
// Template 列表项
//
// 职责：
//   - 单条模板的摘要展示：名称 + 描述 + 变量数 + 工具数
//   - 提供「编辑」和「删除」操作按钮

import { computed } from "vue";
import { Button } from "@ice-paw/ui";
import { Pencil, Trash2, FileText } from "lucide-vue-next";
import type { Template } from "../../types";

const props = defineProps<{
  template: Template;
}>();

const emit = defineEmits<{
  edit: [template: Template];
  delete: [template: Template];
}>();

const variableCount = computed<number>(() => props.template.variables.length);
const toolCount = computed<number>(() => props.template.tools.length);

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
  <div class="template-card">
    <div class="template-icon" aria-hidden="true">
      <FileText :size="22" />
    </div>
    <div class="template-body">
      <div class="template-name">{{ props.template.name }}</div>
      <div v-if="props.template.description" class="template-desc">
        {{ props.template.description }}
      </div>
      <div class="template-meta">
        <span v-if="variableCount > 0" class="template-tag">
          变量 × {{ variableCount }}
        </span>
        <span v-if="toolCount > 0" class="template-tag">
          工具 × {{ toolCount }}
        </span>
        <span class="template-time">{{ formatDate(props.template.created_at) }}</span>
      </div>
    </div>
    <div class="template-actions">
      <Button
        variant="secondary"
        size="sm"
        :title="`编辑 ${props.template.name}`"
        :aria-label="`编辑 ${props.template.name}`"
        @click="emit('edit', props.template)"
      >
        <template #icon-left>
          <Pencil :size="14" aria-hidden="true" />
        </template>
        编辑
      </Button>
      <Button
        variant="ghost"
        size="sm"
        :title="`删除 ${props.template.name}`"
        :aria-label="`删除 ${props.template.name}`"
        @click="emit('delete', props.template)"
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
.template-card {
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
.template-card:hover {
  box-shadow: var(--ip-shadow-sm);
  border-color: var(--ip-color-border-strong);
}

.template-icon {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 40px;
  height: 40px;
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-secondary);
}

.template-body {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.template-name {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.template-desc {
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-tertiary);
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
}

.template-meta {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ip-spacing-2);
}

.template-tag {
  display: inline-block;
  padding: 2px var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-relaxed);
  border-radius: var(--ip-radius-sm);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-gray-600);
}

.template-time {
  font-size: var(--ip-text-caption-size);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-tertiary);
}

.template-actions {
  display: flex;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
}
</style>

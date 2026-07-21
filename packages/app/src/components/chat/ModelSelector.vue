<script setup lang="ts">
// 模型选择面板 — 嵌在 IpToolDrawer > Model tab 中
//
// 当前阶段（Phase 1.1 之前）：仅展示当前 Agent 的模型名 + 提示后续将支持切换。
// 待 bridge 提供「列出可用模型」API 后，再扩展为下拉选择器。

import { Cpu } from "lucide-vue-next";

defineProps<{
  /** 当前 Agent 的模型 id（如 'gpt-4o' / 'claude-sonnet-4'） */
  current: string;
  /** Agent 名称（用于提示「请到 Agent 配置中调整」） */
  agentName?: string;
}>();
</script>

<template>
  <div class="model-selector">
    <header class="model-selector__header">
      <Cpu :size="14" aria-hidden="true" />
      <span class="model-selector__title">模型</span>
    </header>

    <div class="model-selector__current" role="status" aria-live="polite">
      <span class="model-selector__label">当前模型</span>
      <span class="model-selector__value">{{ current || '未选择模型' }}</span>
    </div>

    <p class="model-selector__hint">
      模型切换功能将在后续版本推出。<template v-if="agentName">
        目前请到「{{ agentName }}」的 Agent 配置中调整。
      </template>
    </p>
  </div>
</template>

<style scoped>
.model-selector {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-3);
}

.model-selector__header {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.model-selector__title {
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}

.model-selector__current {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
  padding: var(--ip-spacing-3);
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-sm);
  border: 1px solid var(--ip-color-border-default);
}

.model-selector__label {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.model-selector__value {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  font-family: var(--ip-font-mono);
  word-break: break-all;
}

.model-selector__hint {
  margin: 0;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.5;
}
</style>

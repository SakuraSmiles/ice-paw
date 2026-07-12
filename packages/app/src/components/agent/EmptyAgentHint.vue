<script setup lang="ts">
// 空状态提示组件
//
// 职责：
//   - Agent 列表为空时显示欢迎引导
//   - 提供「创建 Agent」入口按钮（@ice-paw/ui Button + lucide Plus 图标）
//
// emits:
//   - create: 点击创建按钮时触发

import { Button } from "@ice-paw/ui";
import { Plus } from "lucide-vue-next";

const emit = defineEmits<{
  create: [];
}>();
</script>

<template>
  <div class="empty-hint">
    <h2 class="empty-title">欢迎使用 IcePaw</h2>
    <p class="empty-desc">当前还没有 Agent，创建你的第一个 Agent 开始对话。</p>
    <Button variant="primary" size="md" @click="emit('create')">
      <template #icon-left>
        <Plus :size="16" aria-hidden="true" />
      </template>
      创建 Agent
    </Button>
  </div>
</template>

<style scoped>
.empty-hint {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: var(--ip-spacing-12) var(--ip-spacing-6);
  text-align: center;
  gap: var(--ip-spacing-4);
}

.empty-title {
  margin: 0;
  font-size: var(--ip-text-h2-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
}

.empty-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
  max-width: 320px;
}
</style>
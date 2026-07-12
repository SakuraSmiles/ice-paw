<script setup lang="ts">
// 新建会话按钮
//
// 职责：
//   - 简单按钮：「+ 新建会话」
//   - 点击 emit 'create'（外层接住后调 conversationsStore.create）
//
// emits:
//   - create  点击按钮时触发

import { Plus } from "lucide-vue-next";

const emit = defineEmits<{
  create: [];
}>();
</script>

<template>
  <button class="new-chat-btn" type="button" @click="emit('create')">
    <Plus :size="16" class="new-chat-btn-icon" aria-hidden="true" />
    <span class="new-chat-btn-label">新建会话</span>
  </button>
</template>

<style scoped>
.new-chat-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--ip-spacing-2);
  width: 100%;
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.new-chat-btn:hover {
  background: var(--ip-color-bg-elevated);
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-link);
}

.new-chat-btn:active {
  background: var(--ip-color-bg-tertiary);
}

.new-chat-btn:focus-visible {
  outline: none;
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.new-chat-btn-icon {
  flex-shrink: 0;
  color: currentColor;
}

.new-chat-btn-label {
  line-height: 1;
}
</style>

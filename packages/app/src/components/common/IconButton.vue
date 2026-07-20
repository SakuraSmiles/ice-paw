<script setup lang="ts">
// 通用图标按钮（36×36）
// 轻量包装：lucide icon + aria-label + disabled 态
// 颜色全部走 --ip-* token，适配亮/暗主题

import type { FunctionalComponent } from "vue";

defineProps<{
  icon: FunctionalComponent;
  label: string;
  disabled?: boolean;
}>();

defineEmits<{
  click: [];
}>();
</script>

<template>
  <button
    type="button"
    class="icon-btn"
    :class="{ 'icon-btn--disabled': disabled }"
    :disabled="disabled"
    :aria-label="label"
    :title="label"
    @click="$emit('click')"
  >
    <component :is="icon" :size="18" aria-hidden="true" />
  </button>
</template>

<style scoped>
.icon-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  border-radius: 8px;
  background: transparent;
  border: none;
  cursor: pointer;
  color: var(--ip-color-text-secondary, currentColor);
  appearance: none;
  outline: none;
  transition:
    background-color var(--ip-duration-fast, 120ms) var(--ip-ease-out, ease),
    color var(--ip-duration-fast, 120ms) var(--ip-ease-out, ease);
}

.icon-btn:hover {
  background: var(--ip-color-bg-secondary, rgba(255, 255, 255, 0.08));
  color: var(--ip-color-text-primary, currentColor);
}

.icon-btn:focus-visible {
  box-shadow: var(--ip-shadow-focus, 0 0 0 2px rgba(125, 211, 252, 0.4));
}

.icon-btn:active {
  transform: translateY(0.5px);
}

.icon-btn--disabled {
  opacity: 0.4;
  cursor: not-allowed;
  pointer-events: none;
}
</style>

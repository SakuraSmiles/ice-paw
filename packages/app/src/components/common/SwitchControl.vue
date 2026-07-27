<script setup lang="ts">
// 开关控件（Switch / Toggle）
//
// 设计要点：
// - 简单双向绑定：`v-model:modelValue` 即 boolean。
// - 支持 disabled。
// - a11y：role="switch" + aria-checked + aria-label。
// - 键盘可达：原生 button + Enter/Space。
// - 动画：开关滑动 + 背景色过渡，使用 design token 的 transition。
//
// props:
//   - modelValue: 当前布尔值
//   - disabled?:  是否禁用
//   - label?:     无障碍标签（默认沿用父组件 SettingRow 的 label）

const props = defineProps<{
  modelValue: boolean;
  disabled?: boolean;
  label?: string;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: boolean];
}>();

function toggle(): void {
  if (props.disabled) return;
  emit("update:modelValue", !props.modelValue);
}

function onKeydown(e: KeyboardEvent): void {
  if (props.disabled) return;
  if (e.key === " " || e.key === "Enter") {
    e.preventDefault();
    toggle();
  }
}
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :aria-label="label ?? '开关'"
    :disabled="disabled"
    :class="['ip-switch', { 'ip-switch-on': modelValue, 'ip-switch-disabled': disabled }]"
    @click="toggle"
    @keydown="onKeydown"
  >
    <span class="ip-switch-thumb" aria-hidden="true" />
  </button>
</template>

<style scoped>
.ip-switch {
  --switch-w: 36px;
  --switch-h: 20px;
  --switch-thumb: 16px;
  --switch-off-bg: var(--ip-color-bg-tertiary);
  --switch-on-bg: var(--ip-primary-500);
  --switch-disabled-opacity: 0.5;

  position: relative;
  display: inline-flex;
  align-items: center;
  width: var(--switch-w);
  height: var(--switch-h);
  padding: 0;
  border: none;
  border-radius: 999px;
  background: var(--switch-off-bg);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.ip-switch:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus, 0 0 0 2px rgba(59, 130, 246, 0.4));
}

.ip-switch-on {
  background: var(--switch-on-bg);
}

.ip-switch-disabled {
  opacity: var(--switch-disabled-opacity);
  cursor: not-allowed;
}

.ip-switch-thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: var(--switch-thumb);
  height: var(--switch-thumb);
  background: #ffffff;
  border-radius: 50%;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.18);
  transition: transform var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.ip-switch-on .ip-switch-thumb {
  transform: translateX(calc(var(--switch-w) - var(--switch-thumb) - 4px));
}
</style>
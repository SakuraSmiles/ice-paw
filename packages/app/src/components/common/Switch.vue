<script setup lang="ts">
// Switch.vue — 轻量开关组件
const props = defineProps<{ modelValue: boolean; disabled?: boolean }>();
const emit = defineEmits<{ "update:modelValue": [value: boolean] }>();

function toggle() {
  if (props.disabled) return;
  emit("update:modelValue", !props.modelValue);
}
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    class="switch"
    :class="{ on: modelValue, disabled: disabled }"
    :disabled="disabled"
    @click="toggle"
  >
    <span class="switch-thumb" />
  </button>
</template>

<style scoped>
.switch {
  position: relative;
  width: 34px;
  height: 18px;
  border-radius: var(--ip-radius-full);
  border: none;
  cursor: pointer;
  background-color: var(--ip-color-border-default);
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
  flex-shrink: 0;
  padding: 0;
}
.switch.on { background-color: var(--ip-primary-500); }
.switch.disabled { opacity: 0.5; cursor: not-allowed; }

.switch-thumb {
  position: absolute;
  top: 2px;
  left: 2px;
  width: 14px;
  height: 14px;
  border-radius: 50%;
  background-color: #fff;
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
  box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2);
}
.switch.on .switch-thumb { transform: translateX(16px); }
</style>

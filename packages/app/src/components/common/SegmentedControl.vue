<script setup lang="ts">
// 分段选择器（类似 iOS SegmentedControl）

defineProps<{
  modelValue: string;
  options: { label: string; value: string }[];
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();
</script>

<template>
  <div class="segmented">
    <button
      v-for="opt in options"
      :key="opt.value"
      type="button"
      :class="['segmented-item', { active: modelValue === opt.value }]"
      @click="emit('update:modelValue', opt.value)"
    >
      {{ opt.label }}
    </button>
  </div>
</template>

<style scoped>
.segmented {
  display: inline-flex;
  align-items: center;
  gap: 2px;
  padding: 2px;
  background-color: var(--ip-color-bg-secondary);
  border-radius: 8px;
}

.segmented-item {
  padding: var(--ip-spacing-1) var(--ip-spacing-3);
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  line-height: 1.4;
  color: var(--ip-color-text-secondary);
  background: transparent;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  transition: var(--ip-transition-colors);
  white-space: nowrap;
}

.segmented-item:hover {
  color: var(--ip-color-text-primary);
}

.segmented-item.active {
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-primary);
  box-shadow: 0 1px 3px var(--ip-color-shadow);
}
</style>

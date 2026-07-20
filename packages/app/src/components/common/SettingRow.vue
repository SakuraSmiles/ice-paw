<script setup lang="ts">
// 通用设置行组件
defineProps<{
  label: string;
  description?: string;
}>();
</script>

<template>
  <div class="setting-row">
    <div class="setting-row-info">
      <div class="setting-row-label">{{ label }}</div>
      <div v-if="description" class="setting-row-desc">{{ description }}</div>
    </div>
    <div class="setting-row-control">
      <slot />
    </div>
  </div>
</template>

<style scoped>
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 48px;
  padding: var(--ip-spacing-3) 0;
  gap: var(--ip-spacing-4);
}
.setting-row-info { flex: 1; min-width: 0; }
.setting-row-label {
  font-size: var(--ip-text-body-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  line-height: 1.4;
}
.setting-row-desc {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
  margin-top: var(--ip-spacing-0_5);
}
.setting-row-control { flex-shrink: 0; display: flex; align-items: center; }
</style>

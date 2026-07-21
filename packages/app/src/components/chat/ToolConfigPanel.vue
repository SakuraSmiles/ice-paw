<script setup lang="ts">
// 工具勾选面板 — 嵌在 IpToolDrawer > Tools tab 中
//
// 与 ToolPopover.vue 逻辑相同，仅去掉 popover 浮层包装。
// 职责：
//   - 显示当前 Agent 配置的可选工具列表（多选 checkbox）
//   - 维护对话级覆盖（所有工具都勾选 → 等价于 null = 继承 Agent 配置）
//   - emit `update:toolOverride` 给 ChatInput，ChatInput 再传给 bridge

import { computed, ref, watch } from "vue";
import { Wrench } from "lucide-vue-next";

const props = defineProps<{
  availableTools: string[];
  toolOverride: Record<string, boolean> | null;
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:toolOverride": [value: Record<string, boolean> | null];
}>();

// 内置工具的友好名称（与 ToolPopover 保持一致）
const TOOL_LABELS: Record<string, string> = {
  read_file: "读取文件",
  list_directory: "列出目录",
};

/** 本地勾选状态（独立于 props 以便用户编辑后即时显示） */
const local = ref<Record<string, boolean>>({});

function syncLocal(): void {
  if (props.toolOverride) {
    local.value = { ...props.toolOverride };
  } else {
    const next: Record<string, boolean> = {};
    for (const t of props.availableTools) {
      next[t] = true;
    }
    local.value = next;
  }
}

watch(
  () => [props.toolOverride, props.availableTools] as const,
  syncLocal,
  { immediate: true },
);

const rows = computed(() =>
  props.availableTools.map((name) => ({
    name,
    label: TOOL_LABELS[name] ?? name,
    checked: local.value[name] ?? false,
  })),
);

function toggle(name: string): void {
  local.value[name] = !local.value[name];
  // 检查是否与默认值相同（全勾 = 继承 Agent 配置 = null）
  const isDefault = props.availableTools.every((t) => local.value[t] === true);
  emit("update:toolOverride", isDefault ? null : { ...local.value });
}
</script>

<template>
  <div class="tool-config-panel" :aria-disabled="disabled">
    <header class="tool-config-panel__header">
      <Wrench :size="14" aria-hidden="true" />
      <span class="tool-config-panel__title">工具</span>
      <span class="tool-config-panel__hint">仅本次对话生效</span>
    </header>

    <div v-if="rows.length > 0" class="tool-config-panel__list" role="group">
      <label
        v-for="row in rows"
        :key="row.name"
        class="tool-config-panel__row"
      >
        <input
          type="checkbox"
          class="tool-config-panel__checkbox"
          :checked="row.checked"
          :disabled="disabled"
          @change="toggle(row.name)"
        />
        <span class="tool-config-panel__label">{{ row.label }}</span>
      </label>
    </div>

    <div v-else class="tool-config-panel__empty">未配置工具</div>
  </div>
</template>

<style scoped>
.tool-config-panel {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.tool-config-panel[aria-disabled='true'] {
  opacity: 0.6;
  pointer-events: none;
}

.tool-config-panel__header {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.tool-config-panel__title {
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
}

.tool-config-panel__hint {
  margin-left: var(--ip-spacing-2);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.tool-config-panel__list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-1);
}

.tool-config-panel__row {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  padding: 4px 0;
  cursor: pointer;
  user-select: none;
}

.tool-config-panel__checkbox {
  appearance: none;
  width: 16px;
  height: 16px;
  border: 1.5px solid var(--ip-color-border-strong);
  border-radius: 4px;
  background: transparent;
  cursor: pointer;
  position: relative;
  flex-shrink: 0;
  margin: 0;
  transition: var(--ip-transition-colors);
}

.tool-config-panel__checkbox:checked {
  background: var(--ip-primary-600);
  border-color: var(--ip-primary-600);
}

.tool-config-panel__checkbox:checked::after {
  content: '';
  position: absolute;
  left: 4px;
  top: 1px;
  width: 5px;
  height: 9px;
  border: solid white;
  border-width: 0 2px 2px 0;
  transform: rotate(45deg);
}

.tool-config-panel__checkbox:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.tool-config-panel__checkbox:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.tool-config-panel__label {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  line-height: 1.4;
}

.tool-config-panel__empty {
  padding: var(--ip-spacing-3);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}
</style>

<script setup lang="ts">
// 工具 Popover — per-tool 勾选
//
// 显示当前 Agent 配置的可选工具列表，允许对话级覆盖。
// - 点击外部或「完成」按钮关闭
// - 底部提示「仅本次对话」
// - 所有样式使用 --ip-* design token

import { computed, ref, watch } from "vue";
import { onClickOutside } from "@vueuse/core";

const props = defineProps<{
  /** Agent enabled_tools（或全部内置工具） */
  availableTools: string[];
  /** 对话级覆盖（null = 继承 Agent 配置） */
  toolOverride: Record<string, boolean> | null;
}>();

const emit = defineEmits<{
  "update:toolOverride": [value: Record<string, boolean> | null];
  close: [];
}>();

// 内置工具的友好名称
const TOOL_LABELS: Record<string, string> = {
  read_file: "读取文件",
  list_directory: "列出目录",
};

/** Popover 容器 DOM 引用（onClickOutside 用） */
const containerRef = ref<HTMLElement | null>(null);

onClickOutside(containerRef, () => {
  emit("close");
});

/**
 * 本地勾选状态。
 * - toolOverride 为 null（继承 Agent 配置）时，所有工具默认勾选
 * - toolOverride 非 null 时，使用其中的值
 */
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

// 初始化 + toolOverride 变化时重新同步
watch(
  () => [props.toolOverride, props.availableTools] as const,
  syncLocal,
  { immediate: true },
);

/** 工具行（label + 是否勾选） */
const toolRows = computed(() =>
  props.availableTools.map((name) => ({
    name,
    label: TOOL_LABELS[name] ?? name,
    checked: local.value[name] ?? false,
  })),
);

function toggle(name: string): void {
  local.value[name] = !local.value[name];
}

function done(): void {
  // 检查是否和默认值不同（全选 = 继承 Agent 配置 → null）
  const isDefault = props.availableTools.every((t) => local.value[t] === true);
  emit("update:toolOverride", isDefault ? null : { ...local.value });
  emit("close");
}
</script>

<template>
  <div ref="containerRef" class="tool-popover" role="dialog" aria-label="工具选择">
    <!-- 标题行 -->
    <div class="popover-header">
      <span class="popover-title">工具</span>
      <button class="btn-done" type="button" @click="done">完成</button>
    </div>

    <!-- 工具列表 -->
    <div v-if="toolRows.length > 0" class="tool-list">
      <label v-for="row in toolRows" :key="row.name" class="tool-row">
        <input
          type="checkbox"
          class="tool-checkbox"
          :checked="row.checked"
          @change="toggle(row.name)"
        />
        <span class="tool-label">{{ row.label }}</span>
      </label>
    </div>

    <!-- 空状态 -->
    <div v-else class="empty-hint">未配置工具</div>

    <!-- 底部提示 -->
    <div class="popover-footer">仅本次对话</div>
  </div>
</template>

<style scoped>
.tool-popover {
  width: 240px;
  border-radius: 12px;
  background: var(--ip-color-bg-primary);
  box-shadow: var(--ip-shadow-popover, 0 4px 16px rgba(0, 0, 0, 0.12));
  border: 1px solid var(--ip-color-border-default);
  overflow: hidden;
  z-index: 100;
}

.popover-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px 6px;
}

.popover-title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
}

.btn-done {
  appearance: none;
  border: none;
  background: transparent;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-primary-600);
  cursor: pointer;
  padding: 2px 6px;
  border-radius: var(--ip-radius-sm);
  transition: var(--ip-transition-colors);
}

.btn-done:hover {
  color: var(--ip-primary-700);
  background: var(--ip-primary-100, #dbeafe);
}

.tool-list {
  padding: 4px 12px;
  max-height: 240px;
  overflow-y: auto;
}

.tool-row {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 0;
  cursor: pointer;
  user-select: none;
}

.tool-checkbox {
  appearance: none;
  width: 16px;
  height: 16px;
  border: 1.5px solid var(--ip-color-border-strong);
  border-radius: 4px;
  background: transparent;
  cursor: pointer;
  position: relative;
  flex-shrink: 0;
  transition: var(--ip-transition-colors);
}

.tool-checkbox:checked {
  background: var(--ip-primary-600);
  border-color: var(--ip-primary-600);
}

.tool-checkbox:checked::after {
  content: "";
  position: absolute;
  left: 4px;
  top: 1px;
  width: 5px;
  height: 9px;
  border: solid white;
  border-width: 0 2px 2px 0;
  transform: rotate(45deg);
}

.tool-label {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  line-height: 1.4;
}

.empty-hint {
  padding: 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}

.popover-footer {
  padding: 6px 12px 10px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  border-top: 1px solid var(--ip-color-border-default);
}
</style>

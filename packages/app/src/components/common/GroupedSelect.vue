<script setup lang="ts">
// GroupedSelect.vue — 分组选择器（el-select + el-option-group 风格）
//
// 纯选择语义：关闭态是 selector（无光标输入），展开后按组浏览/搜索过滤，
// 只能选中条目级选项——组头是 optgroup 式纯标签，不可点选。与 Combobox
// （可选可输）互补：模型目录这类「合法值有限」的选择用它。
//
// - 条目 value 全局唯一（调用方编码，如 `provider::model`），选中 emit
//   `select` 携带完整条目（含 data 负载）+ update:modelValue
// - 搜索：展开态顶部过滤框，条目匹配 label/value；组头命中保留整组
// - 组尾插槽 `group-extra`（参数 group）：组内常驻自定义内容（如自定义
//   组的模型名输入框）
import { ref, computed, onMounted, onUnmounted, nextTick } from "vue";
import type { ComboboxGroup, ComboboxItem } from "./Combobox.vue";

const props = withDefaults(defineProps<{
  modelValue: string;
  groups: ComboboxGroup[];
  placeholder?: string;
  disabled?: boolean;
}>(), {
  placeholder: "",
  disabled: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  select: [item: ComboboxItem];
}>();

const root = ref<HTMLElement | null>(null);
const search = ref<HTMLInputElement | null>(null);
const open = ref(false);
const query = ref("");

const flatItems = computed<ComboboxItem[]>(() =>
  props.groups.flatMap((g) => g.items),
);

const selectedLabel = computed(() => {
  const hit = flatItems.value.find((it) => it.value === props.modelValue);
  return hit?.label ?? "";
});

/** 过滤后的组（组头命中 → 整组保留；组头不可选，仅作浏览锚点） */
const filteredGroups = computed(() => {
  if (!query.value) return props.groups.map((g) => ({ g, matched: g.items }));
  const q = query.value.toLowerCase();
  return props.groups
    .map((g) => {
      const headHit =
        g.label.toLowerCase().includes(q) || (g.note ?? "").toLowerCase().includes(q);
      const itemHits = g.items.filter(
        (it) => it.label.toLowerCase().includes(q) || it.value.toLowerCase().includes(q),
      );
      return { g, matched: headHit ? g.items : itemHits };
    })
    .filter(({ g, matched }) => matched.length > 0 || g.items.length === 0);
});

/** 搜索无结果时组尾插槽（自定义输入框）仍可用——组非空才参与过滤链 */
const hasVisible = computed(() =>
  filteredGroups.value.some(({ matched }) => matched.length > 0),
);

function onDocClick(e: MouseEvent) {
  if (open.value && root.value && !root.value.contains(e.target as Node)) {
    open.value = false;
    query.value = "";
  }
}
onMounted(() => document.addEventListener("mousedown", onDocClick));
onUnmounted(() => document.removeEventListener("mousedown", onDocClick));

async function toggle() {
  if (props.disabled) return;
  open.value = !open.value;
  query.value = "";
  if (open.value) await nextTick();
  search.value?.focus();
}

function pick(item: ComboboxItem) {
  emit("select", item);
  emit("update:modelValue", item.value);
  open.value = false;
  query.value = "";
}

function onSearchKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    open.value = false;
    query.value = "";
  }
  if (e.key === "Enter") {
    const first = filteredGroups.value.find(({ matched }) => matched.length > 0)?.matched[0];
    if (first) pick(first);
    e.preventDefault();
  }
}
</script>

<template>
  <div ref="root" class="gs" :class="{ 'gs-open': open, 'gs-disabled': disabled }">
    <!-- 关闭态：selector 控件（无光标输入） -->
    <button type="button" class="gs-control" :disabled="disabled" @click="toggle">
      <span class="gs-value" :class="{ 'gs-placeholder': !selectedLabel }">
        {{ selectedLabel || placeholder }}
      </span>
      <svg class="gs-chevron" :class="{ rotated: open }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <polyline points="6 9 12 15 18 9" />
      </svg>
    </button>

    <!-- 展开态：搜索 + 分组列表 -->
    <div v-if="open" class="gs-dropdown">
      <div class="gs-search-row">
        <input
          ref="search"
          v-model="query"
          type="text"
          class="gs-search"
          placeholder="搜索模型…"
          @keydown="onSearchKeydown"
        />
      </div>
      <div class="gs-list">
        <template v-for="{ g, matched } in filteredGroups" :key="g.label">
          <div class="gs-group-label">
            <span class="gs-group-name">{{ g.label }}</span>
            <span v-if="g.note" class="gs-group-note">{{ g.note }}</span>
          </div>
          <button
            v-for="it in matched"
            :key="it.value"
            type="button"
            :class="['gs-option', { active: it.value === modelValue }]"
            @click="pick(it)"
          >
            <span class="gs-option-label">{{ it.label }}</span>
            <span v-if="it.note" class="gs-option-note">{{ it.note }}</span>
          </button>
          <slot name="group-extra" :group="g" />
        </template>
        <div v-if="!hasVisible && !$slots['group-extra']" class="gs-empty">无匹配模型</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.gs {
  position: relative;
  width: 100%;
}

.gs-control {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  height: 30px;
  padding: 0 4px 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.gs-control:hover {
  border-color: var(--ip-primary-300);
}
.gs-open .gs-control,
.gs-control:focus-visible {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}
.gs-disabled .gs-control {
  opacity: 0.6;
  cursor: not-allowed;
}

.gs-value {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
}
.gs-placeholder {
  color: var(--ip-color-text-placeholder);
}

.gs-chevron {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}
.gs-chevron.rotated {
  transform: rotate(180deg);
}

.gs-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  z-index: 100;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  overflow: hidden;
}

.gs-search-row {
  padding: 6px;
  border-bottom: 1px solid var(--ip-color-border-default);
}
.gs-search {
  width: 100%;
  height: 26px;
  padding: 0 8px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  outline: none;
  box-sizing: border-box;
}
.gs-search:focus {
  border-color: var(--color-input-focus-border);
}
.gs-search::placeholder {
  color: var(--ip-color-text-placeholder);
}

.gs-list {
  max-height: 264px;
  overflow-y: auto;
  padding: 4px;
}

.gs-group-label {
  display: flex;
  align-items: baseline;
  gap: 6px;
  padding: 8px 10px 3px;
}
.gs-group-name {
  font-size: 11px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.02em;
}
.gs-group-note {
  font-size: 10px;
  font-weight: var(--ip-font-weight-regular);
  color: var(--ip-color-text-disabled);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.gs-option {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  width: 100%;
  padding: 6px 10px 6px 20px;
  text-align: left;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.gs-option:hover {
  background-color: var(--color-sidebar-item-hover);
}
.gs-option.active {
  background-color: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary);
}
.gs-option-label {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.gs-option-note {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--ip-color-text-tertiary);
}
.gs-option.active .gs-option-note {
  color: var(--ip-color-text-on-primary);
  opacity: 0.85;
}

.gs-empty {
  padding: 12px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}
</style>

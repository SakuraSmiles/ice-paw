<script setup lang="ts">
// GroupedSelect.vue — 分组可选可输选择器（el-select filterable + optgroup 风格）
//
// 控件本身就是输入框：输入实时过滤下拉（条目命中只留命中项、组头命中保留
// 整组），展开态不再有独立搜索框。组头是 optgroup 式纯标签不可点，只有条目
// 级可点选——「合法值有限但允许目录外名字」（模型目录）用它。
//
// - 条目 value 全局唯一（调用方编码，如 `provider::model`），选中 emit
//   `select` 携带完整条目（含 data 负载）+ update:modelValue
// - 显示值：modelValue 命中条目显 label；否则显 unmatchedLabel（受控方给，
//   如手输模型名）；输入草稿只在展开态短暂存在，点外/Esc 丢弃不落地
// - allowCustom：输入无精确命中时列表底部出现「使用 “输入名”」条目——
//   目录外名字的唯一入口（自定义 OpenAI 兼容端点 / Ollama 本地模型）
// - 组头插槽 `group-icon`（参数 group）：组名前的品牌图标位
// - 控件插槽 `control-icon`：控件前缀（如当前选中条目的品牌图标）
import { ref, computed, onMounted, onUnmounted, nextTick, watch } from "vue";
import type { ComboboxGroup, ComboboxItem } from "./Combobox.vue";

const props = withDefaults(defineProps<{
  modelValue: string;
  groups: ComboboxGroup[];
  placeholder?: string;
  disabled?: boolean;
  /** 目录外名字入口：输入无精确命中时提供「使用 “输入名”」条目 */
  allowCustom?: boolean;
  /** modelValue 无条目命中时的显示值（手输模型名由受控方回显） */
  unmatchedLabel?: string;
}>(), {
  placeholder: "",
  disabled: false,
  allowCustom: false,
  unmatchedLabel: "",
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  select: [item: ComboboxItem];
}>();

const root = ref<HTMLElement | null>(null);
const input = ref<HTMLInputElement | null>(null);
const open = ref(false);
/** 用户已实际输入（区别于程序回显当前值）——展开初期显示全目录，键入才开始过滤 */
const filtering = ref(false);
const draft = ref("");

const flatItems = computed<ComboboxItem[]>(() =>
  props.groups.flatMap((g) => g.items),
);

/** 关闭态显示值：命中条目 label，否则 unmatchedLabel（手输名），再退 placeholder */
const displayLabel = computed(() => {
  const hit = flatItems.value.find((it) => it.value === props.modelValue);
  return hit?.label ?? props.unmatchedLabel;
});

// 初始草稿 = 显示值（watch 只在关闭态跟随受控显示值——展开时用户正在输入；
// watch displayLabel 而非 modelValue：custom 落地前后 modelValue 都是空串不触发）
draft.value = displayLabel.value;
watch(displayLabel, () => {
  if (!open.value) draft.value = displayLabel.value;
});

/** 过滤后的组（组头命中 → 整组保留；组头不可选，仅作浏览锚点） */
const filteredGroups = computed(() => {
  if (!filtering.value || !draft.value) return props.groups.map((g) => ({ g, matched: g.items }));
  const q = draft.value.toLowerCase();
  return props.groups
    .map((g) => {
      const headHit =
        g.label.toLowerCase().includes(q) || (g.note ?? "").toLowerCase().includes(q);
      const itemHits = g.items.filter(
        (it) => it.label.toLowerCase().includes(q) || it.value.toLowerCase().includes(q),
      );
      return { g, matched: headHit ? g.items : itemHits };
    })
    .filter(({ matched }) => matched.length > 0);
});

const hasVisible = computed(() =>
  filteredGroups.value.some(({ matched }) => matched.length > 0),
);

/** 精确命中（大小写不敏感）抑制 custom 条目——模糊命中仍是目录外名字（逃生口保留） */
const exactHit = computed(() => {
  const q = draft.value.trim().toLowerCase();
  return !!q && flatItems.value.some((it) => it.label.toLowerCase() === q);
});

const customItem = computed<ComboboxItem | null>(() => {
  if (!props.allowCustom || !open.value || !filtering.value || !draft.value.trim() || exactHit.value) return null;
  const text = draft.value.trim();
  return { label: text, value: `custom::${text}`, data: { custom: true, model: text } };
});

/** 草稿归位显示值（关闭下拉时丢弃未落地的输入） */
function syncDraft() {
  filtering.value = false;
  draft.value = displayLabel.value;
}

function onDocClick(e: MouseEvent) {
  if (open.value && root.value && !root.value.contains(e.target as Node)) {
    open.value = false;
    syncDraft();
  }
}
onMounted(() => document.addEventListener("mousedown", onDocClick));
onUnmounted(() => document.removeEventListener("mousedown", onDocClick));

async function onInputFocus() {
  if (props.disabled) return;
  if (!open.value) {
    open.value = true;
    syncDraft();
    await nextTick();
    input.value?.select(); // 全选：直接键入即覆盖，改一个字母的场合少数
  }
}

function toggle() {
  if (props.disabled) return;
  if (open.value) {
    open.value = false;
    syncDraft();
  } else {
    input.value?.focus();
  }
}

function pick(item: ComboboxItem) {
  emit("select", item);
  emit("update:modelValue", item.value);
  open.value = false;
  syncDraft();
}

function onKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    open.value = false;
    syncDraft();
  } else if (e.key === "Enter") {
    // 优先级：第一个目录命中条目 > custom 兜底（完全无命中时才落到目录外名字）
    const first = filteredGroups.value.find(({ matched }) => matched.length > 0)?.matched[0];
    if (first) pick(first);
    else if (customItem.value) pick(customItem.value);
    e.preventDefault();
  }
}
</script>

<template>
  <div ref="root" class="gs" :class="{ 'gs-open': open, 'gs-disabled': disabled }">
    <!-- 控件即输入框：输入实时过滤；chevron 切换展开/收起 -->
    <div class="gs-control">
      <slot name="control-icon" />
      <input
        ref="input"
        v-model="draft"
        type="text"
        class="gs-input"
        :placeholder="placeholder"
        :disabled="disabled"
        @focus="onInputFocus"
        @input="filtering = true"
        @keydown="onKeydown"
      />
      <svg class="gs-chevron" :class="{ rotated: open }" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" @click="toggle">
        <polyline points="6 9 12 15 18 9" />
      </svg>
    </div>

    <!-- 展开态：分组列表（搜索即控件输入本身，无独立搜索框） -->
    <div v-if="open" class="gs-dropdown">
      <div class="gs-list">
        <template v-for="{ g, matched } in filteredGroups" :key="g.label">
          <div class="gs-group-label">
            <slot name="group-icon" :group="g" />
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
        </template>
        <!-- 目录外名字入口：点它 = 以输入名落自定义（data.custom） -->
        <button
          v-if="customItem"
          type="button"
          class="gs-option gs-option-custom"
          @click="pick(customItem)"
        >
          <span class="gs-option-label">使用自定义模型 “{{ customItem.label }}”</span>
          <span class="gs-option-note">手动填写 API URL</span>
        </button>
        <div v-if="!hasVisible && !customItem" class="gs-empty">无匹配模型</div>
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
  gap: 8px;
  width: 100%;
  height: 30px;
  padding: 0 4px 0 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  cursor: text;
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}
.gs-control:hover {
  border-color: var(--ip-primary-300);
}
.gs-open .gs-control {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
}
.gs-disabled .gs-control {
  opacity: 0.6;
  cursor: not-allowed;
}

.gs-input {
  flex: 1;
  min-width: 0;
  height: 28px;
  padding: 0;
  font-size: inherit;
  font-family: inherit;
  color: inherit;
  background: transparent;
  border: none;
  outline: none;
}
.gs-input::placeholder {
  color: var(--ip-color-text-placeholder);
}

.gs-chevron {
  flex-shrink: 0;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
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
/* 品牌图标（slot 内容属父作用域，须 :deep）；svg 无基线，垂直居中更稳 */
.gs-group-label :deep(.provider-icon) {
  align-self: center;
}
/* 控件前缀图标：比正文稍收敛的次级色 */
.gs-control :deep(.provider-icon) {
  color: var(--ip-color-text-secondary);
  margin-right: -3px;
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

/* 目录外名字入口条目：与目录条目区分（虚线分隔 + 主色强调） */
.gs-option-custom {
  margin-top: 2px;
  border-top: 1px dashed var(--ip-color-border-default);
  border-radius: 0;
}
.gs-option-custom .gs-option-label {
  color: var(--ip-primary-600);
}

.gs-empty {
  padding: 12px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}
</style>

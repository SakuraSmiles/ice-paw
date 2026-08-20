<script setup lang="ts">
// Combobox.vue — 可选可输的下拉组件
// 设计：输入框 + 过滤下拉列表，支持自由输入
//
// 两种数据形态（互斥，items 优先）：
// - options: string[]           纯字符串（原路径，显示=值=选中值）
// - items: {label,value,note}[] 带元数据：下拉显示 label（+note 副行）、
//   选中 emit value、输入框显示 value 对应 label（无匹配显 value 原文，
//   手输模型名等自由文本保留）。
// 纯选择语义（不可输入）的分组选择器见 GroupedSelect.vue。
import { ref, computed, watch, onUnmounted } from "vue";

export interface ComboboxItem {
  label: string;
  value: string;
  note?: string;
  /** 选中负载：调用方挂任意结构随条目透传（GroupedSelect select 事件用） */
  data?: unknown;
}

export interface ComboboxGroup {
  /** 组标识（如 provider 名）：不渲染、不可选，供调用方在插槽/归属判断用 */
  id?: string;
  /** 组头显示名（如「智谱 GLM」） */
  label: string;
  /** 组头副行说明（如「Coding Plan 专用」） */
  note?: string;
  items: ComboboxItem[];
}

const props = withDefaults(defineProps<{
  modelValue: string;
  options?: string[];
  items?: ComboboxItem[];
  placeholder?: string;
  disabled?: boolean;
}>(), {
  options: () => [],
  items: undefined,
  placeholder: "",
  disabled: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const input = ref<HTMLInputElement | null>(null);
const open = ref(false);
const filter = ref("");
let blurTimer: ReturnType<typeof setTimeout> | null = null;

// value → label 回查（items 形态；手输/无匹配回退 value 原文）
const labelOf = (value: string): string =>
  props.items?.find((it) => it.value === value)?.label ?? value;

// 当前输入框展示的值（受控）：items 形态显示 label，其余显示原值
const displayValue = ref(labelOf(props.modelValue));
// 自身 emit 后的回显标记：父组件 v-model 回写同一值时不算「外部变化」，
// 不重置 filter（否则手输每个字符都会清掉过滤条件，边输边筛失效）
let lastEmitted: string | null = null;
// 跟踪外部 modelValue 变化（父组件可能绕过 Combobox 修改值）
// 同步显示值并重置过滤器，避免下拉菜单基于过时 filter 显示错误选项集
watch(() => props.modelValue, (v) => {
  if (v === lastEmitted) {
    lastEmitted = null;
    return;
  }
  displayValue.value = labelOf(v);
  filter.value = "";
});

// 高亮基准用 modelValue（items 形态 displayValue 是 label，比对会恒 false）
const isActive = (key: string) => key === props.modelValue;

// 过滤：items 形态匹配 label+value；options 形态匹配字符串本身
const filteredItems = computed<ComboboxItem[]>(() => {
  if (!props.items) return [];
  if (!filter.value) return props.items;
  const q = filter.value.toLowerCase();
  return props.items.filter(
    (it) => it.label.toLowerCase().includes(q) || it.value.toLowerCase().includes(q),
  );
});
const filteredOptions = computed<string[]>(() => {
  if (props.items) return [];
  if (!filter.value) return props.options;
  const q = filter.value.toLowerCase();
  return props.options.filter((o) => o.toLowerCase().includes(q));
});
const hasResults = computed(() =>
  props.items ? filteredItems.value.length > 0 : filteredOptions.value.length > 0,
);

function onInput(e: Event) {
  const val = (e.target as HTMLInputElement).value;
  displayValue.value = val;
  filter.value = val;
  // items 形态：手输的是「显示文本」。与某 label 精确一致 → 对应 value；
  // 否则按原文透传（自由输入语义，父组件存原值）
  const matched = props.items?.find((it) => it.label === val);
  lastEmitted = matched ? matched.value : val;
  emit("update:modelValue", lastEmitted);
  if (!open.value) open.value = true;
}

function onFocus() {
  // 清除上一次失焦的延迟关闭定时器，避免重新聚焦后被过时定时器关闭下拉
  if (blurTimer) { clearTimeout(blurTimer); blurTimer = null; }
  filter.value = "";
  open.value = true;
}

function onBlur() {
  // 延迟关闭，让点击 option 的事件先触发
  blurTimer = setTimeout(() => { open.value = false; }, 150);
}

function selectOption(opt: string) {
  displayValue.value = opt;
  filter.value = "";
  lastEmitted = opt;
  emit("update:modelValue", opt);
  open.value = false;
  input.value?.focus();
}

function selectItem(item: ComboboxItem) {
  displayValue.value = item.label;
  filter.value = "";
  lastEmitted = item.value;
  emit("update:modelValue", item.value);
  open.value = false;
  input.value?.focus();
}

function toggle() {
  if (props.disabled) return;
  if (open.value) {
    open.value = false;
  } else {
    filter.value = "";
    open.value = true;
    input.value?.focus();
  }
}

function handleKeydown(e: KeyboardEvent) {
  if (e.key === "Escape") {
    open.value = false;
  }
  if (e.key === "Enter" && open.value && hasResults.value) {
    if (props.items) selectItem(filteredItems.value[0]);
    else selectOption(filteredOptions.value[0]);
    e.preventDefault();
  }
}

// 组件卸载时清除待处理的延迟关闭定时器
onUnmounted(() => {
  if (blurTimer) { clearTimeout(blurTimer); blurTimer = null; }
});
</script>

<template>
  <div class="combobox" :class="{ 'combobox-open': open }">
    <div class="combobox-input-wrap">
      <input
        ref="input"
        :value="displayValue"
        type="text"
        class="combobox-input"
        :placeholder="placeholder"
        :disabled="disabled"
        @input="onInput"
        @focus="onFocus"
        @blur="onBlur"
        @keydown="handleKeydown"
      />
      <button
        type="button"
        class="combobox-chevron"
        :class="{ rotated: open }"
        tabindex="-1"
        aria-label="展开选项"
        @mousedown.prevent
        @click="toggle"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
    </div>

    <!-- items 形态：label 主行 + note 副行，emit value -->
    <div v-if="open && props.items && filteredItems.length > 0" class="combobox-dropdown">
      <button
        v-for="it in filteredItems"
        :key="it.value"
        type="button"
        :class="['combobox-option', 'combobox-option-rich', { active: isActive(it.value) }]"
        @mousedown.prevent
        @click="selectItem(it)"
      >
        <span class="combobox-option-label">{{ it.label }}</span>
        <span v-if="it.note" class="combobox-option-note">{{ it.note }}</span>
      </button>
    </div>
    <!-- options 形态（原路径） -->
    <div v-else-if="open && !props.items && filteredOptions.length > 0" class="combobox-dropdown">
      <button
        v-for="opt in filteredOptions"
        :key="opt"
        type="button"
        :class="['combobox-option', { active: isActive(opt) }]"
        @mousedown.prevent
        @click="selectOption(opt)"
      >
        {{ opt }}
      </button>
    </div>
    <div v-else-if="open && displayValue && !hasResults" class="combobox-dropdown">
      <div class="combobox-empty">无匹配选项</div>
    </div>
  </div>
</template>

<style scoped>
.combobox {
  position: relative;
  width: 100%;
}

.combobox-input-wrap {
  display: flex;
  align-items: center;
  height: 36px;
  padding: 0 0 0 12px;
  background-color: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  transition: all var(--ip-duration-fast) var(--ip-ease-out);
}

.combobox-open .combobox-input-wrap,
.combobox-input-wrap:focus-within {
  border-color: var(--color-input-focus-border);
  background-color: var(--color-input-bg);
  box-shadow: 0 0 0 3px rgba(var(--ip-primary-500-rgb), 0.12);
}

.combobox-input {
  flex: 1;
  min-width: 0;
  border: none;
  outline: none;
  background: transparent;
  padding: 0;
  height: 100%;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  font-family: inherit;
}

.combobox-input::placeholder {
  color: var(--ip-color-text-placeholder);
}

.combobox-input:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.combobox-chevron {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 30px;
  height: 100%;
  background: transparent;
  border: none;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  flex-shrink: 0;
  transition: transform var(--ip-duration-fast) var(--ip-ease-out);
}

.combobox-chevron.rotated {
  transform: rotate(180deg);
}

.combobox-chevron:hover {
  color: var(--ip-color-text-secondary);
}

/* 下拉列表 */
.combobox-dropdown {
  position: absolute;
  top: calc(100% + 4px);
  left: 0;
  right: 0;
  z-index: 100;
  max-height: 200px;
  overflow-y: auto;
  background-color: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  box-shadow: var(--ip-shadow-lg);
  padding: 4px;
}

.combobox-option {
  display: block;
  width: 100%;
  padding: 7px 10px;
  text-align: left;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.combobox-option:hover {
  background-color: var(--color-sidebar-item-hover);
}

.combobox-option.active {
  background-color: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary);
}

/* items 形态：label 主行 + note 副行两行布局 */
.combobox-option-rich {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 6px 10px;
}

.combobox-option-note {
  font-size: 11px;
  color: var(--ip-color-text-tertiary);
  line-height: 1.4;
}

.combobox-option.active .combobox-option-note {
  color: var(--ip-color-text-on-primary);
  opacity: 0.85;
}

.combobox-empty {
  padding: 7px 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}
</style>

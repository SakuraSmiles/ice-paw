<script setup lang="ts">
// Combobox.vue — 可选可输的下拉组件
// 设计：输入框 + 过滤下拉列表，支持自由输入
//
// 三种数据形态（互斥，groups > items > options）：
// - options: string[]           纯字符串（原路径，显示=值=选中值）
// - items: {label,value,note}[] 带元数据：下拉显示 label（+note 副行）、
//   选中 emit value、输入框显示 value 对应 label（无匹配显 value 原文，
//   手输模型名等自由文本保留）。provider 目录下拉用。
// - groups: ComboboxGroup[]     分组形态（模型目录用）：组头（可选时点击=
//   选择该组入口，如 provider）+ 组内条目；条目 value 须全局唯一（调用方
//   自行编码，如 `provider::model`），选中除 update:modelValue 外还 emit
//   `select` 携带完整条目（含 data 负载）。过滤匹配组头时保留整组。
import { ref, computed, watch, onUnmounted } from "vue";

export interface ComboboxItem {
  label: string;
  value: string;
  note?: string;
  /** 选中负载（groups 形态）：调用方挂任意结构，随 select 事件带回 */
  data?: unknown;
}

export interface ComboboxGroup {
  /** 组头显示名（如「智谱 GLM」） */
  label: string;
  /** 组头副行说明（如「Coding Plan 专用」） */
  note?: string;
  /** 存在 = 组头可点击选中（作为该组入口，值通常为组标识/provider 名） */
  headerValue?: string;
  items: ComboboxItem[];
}

const props = withDefaults(defineProps<{
  modelValue: string;
  options?: string[];
  items?: ComboboxItem[];
  groups?: ComboboxGroup[];
  placeholder?: string;
  disabled?: boolean;
}>(), {
  options: () => [],
  items: undefined,
  groups: undefined,
  placeholder: "",
  disabled: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
  /** groups 形态：点击/回车选中条目（含可选组头）时触发，携带完整条目 */
  select: [item: ComboboxItem];
}>();

const input = ref<HTMLInputElement | null>(null);
const open = ref(false);
const filter = ref("");
let blurTimer: ReturnType<typeof setTimeout> | null = null;

/** 可选组头包装为条目（data 标记 isHeader，调用方据此走「选组不选模型」分支） */
const headerItemOf = (g: ComboboxGroup): ComboboxItem => ({
  label: g.label,
  value: g.headerValue ?? "",
  note: g.note,
  data: { isHeader: true },
});

// groups 形态打平（组头+条目），供 labelOf 回查与 Enter 首选
const flatGroupItems = computed<ComboboxItem[]>(() =>
  props.groups
    ? props.groups.flatMap((g) => (g.headerValue ? [headerItemOf(g), ...g.items] : g.items))
    : [],
);

// value → label 回查（items/groups 形态；手输/无匹配回退 value 原文）
const labelOf = (value: string): string =>
  props.groups
    ? flatGroupItems.value.find((it) => it.value === value)?.label ?? value
    : props.items?.find((it) => it.value === value)?.label ?? value;

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

// 过滤：groups 形态（组头命中保留整组）；items 形态匹配 label+value；options 匹配字符串
const filteredGroups = computed<(ComboboxGroup & { matched: ComboboxItem[]; headHit: boolean })[]>(() => {
  const hit = (g: ComboboxGroup, matched: ComboboxItem[], headHit: boolean) => ({ ...g, matched, headHit });
  if (!props.groups) return [];
  if (!filter.value) return props.groups.map((g) => hit(g, g.items, false));
  const q = filter.value.toLowerCase();
  return props.groups
    .map((g) => {
      const headHit =
        g.label.toLowerCase().includes(q) || (g.note ?? "").toLowerCase().includes(q);
      const itemHits = g.items.filter(
        (it) => it.label.toLowerCase().includes(q) || it.value.toLowerCase().includes(q),
      );
      // 组头命中 → 整组条目保留（搜厂商名想看这家全部模型）
      return hit(g, headHit ? g.items : itemHits, headHit);
    })
    // 保留条件：有条目命中，或可选组头自身命中（custom 组无预置条目也可见）
    .filter((g) => g.matched.length > 0 || (g.headerValue !== undefined && g.headHit));
});
const filteredItems = computed<ComboboxItem[]>(() => {
  if (!props.items) return [];
  if (!filter.value) return props.items;
  const q = filter.value.toLowerCase();
  return props.items.filter(
    (it) => it.label.toLowerCase().includes(q) || it.value.toLowerCase().includes(q),
  );
});
const filteredOptions = computed<string[]>(() => {
  if (props.items || props.groups) return [];
  if (!filter.value) return props.options;
  const q = filter.value.toLowerCase();
  return props.options.filter((o) => o.toLowerCase().includes(q));
});
const hasResults = computed(() =>
  props.groups
    ? filteredGroups.value.length > 0
    : props.items
      ? filteredItems.value.length > 0
      : filteredOptions.value.length > 0,
);

function onInput(e: Event) {
  const val = (e.target as HTMLInputElement).value;
  displayValue.value = val;
  filter.value = val;
  if (props.groups) {
    // groups 形态：手输一律原文透传——归属解析是调用方（持有目录）的事
    lastEmitted = val;
    emit("update:modelValue", val);
    if (!open.value) open.value = true;
    return;
  }
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
  // groups 形态：点选只发 select（调用方持有目录，由它全权推导状态）；
  // 不双发 update——编码后的 key 会被调用方的手输通道误读为自由文本
  if (props.groups) {
    emit("select", item);
  } else {
    lastEmitted = item.value;
    emit("update:modelValue", item.value);
  }
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
    if (props.groups) {
      const first = filteredGroups.value[0]?.matched[0];
      if (first) selectItem(first);
    } else if (props.items) selectItem(filteredItems.value[0]);
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

    <!-- groups 形态：可选组头 + 组内条目（模型目录） -->
    <div v-if="open && props.groups && filteredGroups.length > 0" class="combobox-dropdown">
      <template v-for="g in filteredGroups" :key="g.label">
        <!-- 可选组头（provider 入口）：可点击选中；无 headerValue 时纯标签 -->
        <button
          v-if="g.headerValue"
          type="button"
          class="combobox-option combobox-group-head"
          @mousedown.prevent
          @click="selectItem({ label: g.label, value: g.headerValue, note: g.note, data: { isHeader: true } })"
        >
          <span class="combobox-option-label">{{ g.label }}</span>
          <span v-if="g.note" class="combobox-option-note">{{ g.note }}</span>
        </button>
        <div v-else class="combobox-group-label">{{ g.label }}</div>
        <button
          v-for="it in g.matched"
          :key="it.value"
          type="button"
          :class="['combobox-option', 'combobox-option-rich', 'combobox-group-item', { active: isActive(it.value) }]"
          @mousedown.prevent
          @click="selectItem(it)"
        >
          <span class="combobox-option-label">{{ it.label }}</span>
          <span v-if="it.note" class="combobox-option-note">{{ it.note }}</span>
        </button>
      </template>
    </div>
    <!-- items 形态：label 主行 + note 副行，emit value -->
    <div v-else-if="open && props.items && filteredItems.length > 0" class="combobox-dropdown">
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
  box-shadow: 0 0 0 3px rgba(46, 141, 100, 0.12);
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

/* groups 形态：可选组头（provider 入口，厂商名+副行，缩进略浅区分层级） */
.combobox-group-head {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 7px 10px 5px;
  font-weight: var(--ip-font-weight-semibold);
}
.combobox-group-head .combobox-option-label {
  font-size: var(--ip-text-body-sm-size);
}
.combobox-group-head + .combobox-group-item,
.combobox-group-label + .combobox-group-item {
  margin-top: 1px;
}
.combobox-group-item {
  padding-left: 20px;
}
/* 纯标签组头（不可选，仅分组标题） */
.combobox-group-label {
  padding: 8px 10px 3px;
  font-size: 11px;
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-tertiary);
  letter-spacing: 0.02em;
}
.combobox-group-head .combobox-option-note {
  font-weight: var(--ip-font-weight-regular);
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

<script setup lang="ts">
// Combobox.vue — 可选可输的下拉组件
// 设计：输入框 + 过滤下拉列表，支持自由输入
import { ref, computed } from "vue";

const props = withDefaults(defineProps<{
  modelValue: string;
  options: string[];
  placeholder?: string;
  disabled?: boolean;
}>(), {
  placeholder: "",
  disabled: false,
});

const emit = defineEmits<{
  "update:modelValue": [value: string];
}>();

const input = ref<HTMLInputElement | null>(null);
const open = ref(false);
const filter = ref("");

// 当前输入框展示的值（受控）
const displayValue = ref(props.modelValue);

// 过滤后的选项
const filteredOptions = computed(() => {
  if (!filter.value) return props.options;
  const q = filter.value.toLowerCase();
  return props.options.filter((o) => o.toLowerCase().includes(q));
});

function onInput(e: Event) {
  const val = (e.target as HTMLInputElement).value;
  displayValue.value = val;
  filter.value = val;
  emit("update:modelValue", val);
  if (!open.value) open.value = true;
}

function onFocus() {
  filter.value = "";
  open.value = true;
}

function onBlur() {
  // 延迟关闭，让点击 option 的事件先触发
  setTimeout(() => { open.value = false; }, 150);
}

function selectOption(opt: string) {
  displayValue.value = opt;
  filter.value = "";
  emit("update:modelValue", opt);
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
  if (e.key === "Enter" && open.value && filteredOptions.value.length > 0) {
    selectOption(filteredOptions.value[0]);
    e.preventDefault();
  }
}
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
        @mousedown.prevent
        @click="toggle"
        tabindex="-1"
        aria-label="展开选项"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="6 9 12 15 18 9" />
        </svg>
      </button>
    </div>
    <div v-if="open && filteredOptions.length > 0" class="combobox-dropdown">
      <button
        v-for="opt in filteredOptions"
        :key="opt"
        type="button"
        :class="['combobox-option', { active: opt === displayValue }]"
        @mousedown.prevent
        @click="selectOption(opt)"
      >
        {{ opt }}
      </button>
    </div>
    <div v-else-if="open && displayValue && filteredOptions.length === 0" class="combobox-dropdown">
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

.combobox-empty {
  padding: 7px 10px;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-tertiary);
  text-align: center;
}
</style>

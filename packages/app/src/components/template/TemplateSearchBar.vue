<script setup lang="ts">
// 模板搜索框（REQ-TMPL-002）
//
// 职责：
//   - 提供搜索关键词输入框，300ms debounce
//   - 搜索结果由 store.filteredTemplates 在客户端过滤
//   - 复用 AgentSearchBar 的交互模式

import { ref, watch, onBeforeUnmount } from "vue";
import { Search } from "lucide-vue-next";
import { Input } from "@ice-paw/ui";

const props = withDefaults(
  defineProps<{
    value?: string;
    placeholder?: string;
    debounceMs?: number;
    disabled?: boolean;
  }>(),
  {
    value: "",
    placeholder: "搜索模板名称…",
    debounceMs: 300,
    disabled: false,
  },
);

const emit = defineEmits<{
  (e: "update:value", value: string): void;
  (e: "change", value: string): void;
  (e: "clear"): void;
}>();

const inner = ref<string>(props.value);

watch(
  () => props.value,
  (next) => {
    if (next !== inner.value) {
      inner.value = next;
    }
  },
);

function onInput(next: string): void {
  inner.value = next;
  emit("update:value", next);
}

// REQ-TMPL-002：300ms debounce
let timer: ReturnType<typeof setTimeout> | null = null;
watch(
  () => inner.value,
  (next) => {
    if (timer) clearTimeout(timer);
    timer = setTimeout(() => {
      timer = null;
      emit("change", next);
    }, props.debounceMs);
  },
);

function onClear(): void {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
  inner.value = "";
  emit("update:value", "");
  emit("change", "");
  emit("clear");
}

function onKeydown(ev: KeyboardEvent): void {
  if (ev.key === "Escape" && inner.value.length > 0) {
    ev.preventDefault();
    onClear();
  }
}

onBeforeUnmount(() => {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
});
</script>

<template>
  <div class="template-search-bar" role="search">
    <div class="template-search-input-wrap">
      <Search :size="16" class="template-search-icon" aria-hidden="true" />
      <Input
        :model-value="inner"
        :placeholder="placeholder"
        :disabled="disabled"
        clearable
        size="md"
        autocomplete="off"
        aria-label="搜索模板"
        @update:model-value="onInput"
        @clear="onClear"
        @keydown="onKeydown"
      />
    </div>
  </div>
</template>

<style scoped>
.template-search-bar {
  width: 100%;
  margin-bottom: var(--ip-spacing-4);
}

.template-search-input-wrap {
  position: relative;
  display: block;
}

.template-search-icon {
  position: absolute;
  top: 50%;
  left: 12px;
  transform: translateY(-50%);
  color: var(--ip-color-text-tertiary);
  pointer-events: none;
  z-index: 1;
}

.template-search-input-wrap :deep(.ip-input__field) {
  padding-left: 28px;
}
.template-search-input-wrap :deep(input) {
  padding-left: 28px;
}
</style>

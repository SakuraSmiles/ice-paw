<script setup lang="ts">
// Agent 搜索框（REQ-AGENT-020）
//
// 职责：
//   - 提供搜索关键词输入框
//   - 输入时 300ms debounce，仅在停顿后才向上 emit change 事件
//   - 支持一键清空 + 快捷键 Esc
//   - 暴露 v-model:value 让父组件可以双向绑定输入文本（即时回显）
//     真正的「搜索请求」走 @change 事件（debounced）
//
// 与 store.setSearch 的关系：
//   父组件通常这样用：
//     <AgentSearchBar
//       v-model:value="draft"
//       @change="(kw) => agentsStore.setSearch(kw)"
//     />
//   父组件也可以直接把 agentsStore.search 绑到 v-model:value，
//   但那样每次按键都会触发 store.setSearch → 后端请求，失去 debounce 意义。

import { ref, watch } from "vue";
import { Search } from "lucide-vue-next";
import { Input } from "@ice-paw/ui";

const props = withDefaults(
  defineProps<{
    /** 输入框当前值（双向绑定） */
    value?: string;
    /** 占位符 */
    placeholder?: string;
    /** debounce 毫秒数（默认 300ms，符合 REQ-AGENT-020） */
    debounceMs?: number;
    /** 是否禁用 */
    disabled?: boolean;
  }>(),
  {
    value: "",
    placeholder: "搜索 Agent 名称或描述…",
    debounceMs: 300,
    disabled: false,
  },
);

const emit = defineEmits<{
  /** 双向绑定：输入框文本即时变化 */
  (e: "update:value", value: string): void;
  /** debounce 后发出的稳定值（用于触发搜索） */
  (e: "change", value: string): void;
  /** 按 Esc 清空 */
  (e: "clear"): void;
}>();

// 受控值（避免直接修改 props.value 触发 Vue 警告）
const inner = ref<string>(props.value);

// 父组件外部值变化（例如 setCurrent 后重置）时同步到 inner
watch(
  () => props.value,
  (next) => {
    if (next !== inner.value) {
      inner.value = next;
    }
  },
);

// 把本地输入同步给父组件（immediate=true 是初始态也同步一次）
function onInput(next: string): void {
  inner.value = next;
  emit("update:value", next);
}

// debounce：连续 onInput 调用只保留最后一次，停顿 debounceMs 后再 emit change
let timer: ReturnType<typeof setTimeout> | null = null;
watch(
  () => inner.value,
  (next) => {
    if (timer) {
      clearTimeout(timer);
    }
    timer = setTimeout(() => {
      timer = null;
      emit("change", next);
    }, props.debounceMs);
  },
);

// 一键清空：清掉本地输入 + 立即触发一次 change（不等 debounce）
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

// Esc 清空：直接走 onClear 走全套路径
function onKeydown(ev: KeyboardEvent): void {
  if (ev.key === "Escape" && inner.value.length > 0) {
    ev.preventDefault();
    onClear();
  }
}

// 组件卸载时清掉未触发的 timer，避免在已卸载组件上 set state
import { onBeforeUnmount } from "vue";
onBeforeUnmount(() => {
  if (timer) {
    clearTimeout(timer);
    timer = null;
  }
});
</script>

<template>
  <div class="agent-search-bar" role="search">
    <div class="agent-search-input-wrap">
      <Search :size="16" class="agent-search-icon" aria-hidden="true" />
      <Input
        :model-value="inner"
        :placeholder="placeholder"
        :disabled="disabled"
        clearable
        size="md"
        autocomplete="off"
        aria-label="搜索 Agent"
        @update:model-value="onInput"
        @clear="onClear"
        @keydown="onKeydown"
      />
    </div>
  </div>
</template>

<style scoped>
.agent-search-bar {
  width: 100%;
  margin-bottom: var(--ip-spacing-4);
}

.agent-search-input-wrap {
  position: relative;
  display: block;
}

/* 左侧放大镜：absolute 叠在 Input 上方（Input 自带 padding-right） */
.agent-search-icon {
  position: absolute;
  top: 50%;
  left: 12px;
  transform: translateY(-50%);
  color: var(--ip-color-text-tertiary);
  pointer-events: none;
  z-index: 1;
}

/* 给 Input 的内部 input 加 padding-left，避免文字压在图标下
   （UI 库 Input 内部原生 input class 是 .ip-input__field；
   这里用 :deep 穿透 scope，把 padding 加在该原生 input 上）
*/
.agent-search-input-wrap :deep(.ip-input__field) {
  padding-left: 28px;
}
.agent-search-input-wrap :deep(input) {
  padding-left: 28px;
}
</style>
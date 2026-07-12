<script setup lang="ts">
// 行内重命名组件
//
// 职责：
//   - 接管会话标题的编辑态
//   - Enter 确认 → emit commit(title)
//   - Esc / blur 取消 → emit cancel
//   - 进入编辑态时自动 focus 并选中全部文字
//
// props:
//   - modelValue  当前标题（v-model）
//   - editing     是否处于编辑态（true 时渲染 input）
//
// emits:
//   - update:modelValue  v-model 双向绑定（输入时同步到外部）
//   - commit(title)      Enter 提交；空标题视为取消
//   - cancel             Esc / blur 取消

import { ref, watch, nextTick } from "vue";

const props = defineProps<{
  modelValue: string;
  editing: boolean;
}>();

const emit = defineEmits<{
  "update:modelValue": [value: string];
  commit: [title: string];
  cancel: [];
}>();

/** input DOM 引用 */
const inputRef = ref<HTMLInputElement | null>(null);

/** 本地草稿（编辑过程中持有输入值） */
const draft = ref<string>(props.modelValue);

/**
 * 同步外部 modelValue 到本地草稿。
 * 注意：编辑态中不重置草稿，避免用户正在编辑时被覆盖。
 */
watch(
  () => props.modelValue,
  (val) => {
    if (!props.editing) {
      draft.value = val;
    }
  },
);

/**
 * 进入编辑态时：重置草稿 + focus + 全选。
 * immediate: true 让组件挂载时若 editing 已为 true 也能正确初始化。
 */
watch(
  () => props.editing,
  async (editing) => {
    if (editing) {
      draft.value = props.modelValue;
      await nextTick();
      inputRef.value?.focus();
      inputRef.value?.select();
    }
  },
  { immediate: true },
);

/** 输入同步 */
function onInput(e: Event): void {
  const v = (e.target as HTMLInputElement).value;
  draft.value = v;
  emit("update:modelValue", v);
}

/** 键盘事件：Enter 提交 / Esc 取消 */
function onKeydown(e: KeyboardEvent): void {
  if (e.key === "Enter") {
    e.preventDefault();
    const v = draft.value.trim();
    if (v.length === 0) {
      // 空标题视为取消（避免创建无标题会话）
      emit("cancel");
      return;
    }
    emit("commit", v);
  } else if (e.key === "Escape") {
    e.preventDefault();
    emit("cancel");
  }
}

/** 失焦即取消 */
function onBlur(): void {
  emit("cancel");
}
</script>

<template>
  <input
    v-if="editing"
    ref="inputRef"
    class="inline-rename"
    :value="draft"
    type="text"
    :maxlength="100"
    @input="onInput"
    @keydown="onKeydown"
    @blur="onBlur"
    @click.stop
    @dblclick.stop
  />
</template>

<style scoped>
.inline-rename {
  display: block;
  width: 100%;
  padding: var(--ip-spacing-1) 6px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-secondary);
  border: 1px solid var(--ip-color-border-focus);
  border-radius: var(--ip-radius-sm);
  outline: none;
  box-sizing: border-box;
}

.inline-rename:focus {
  border-color: var(--ip-primary-600);
  box-shadow: var(--ip-shadow-focus);
}
</style>
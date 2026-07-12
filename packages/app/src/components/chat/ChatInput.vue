<script setup lang="ts">
// 聊天输入框
//
// 职责：
//   - 多行 textarea，自动增高（最多 6 行高度）
//   - Enter 发送；Shift+Enter / Ctrl+Enter / Meta+Enter 换行
//   - 右下发送按钮：默认显示 SendHorizontal 图标；流式中切换为 Square + 红色高亮
//   - 全部样式走 --ip-input-* Design Token，焦点态使用 --ip-shadow-focus 发光环
//   - 暗色模式：颜色由 token 自动覆盖
//
// props:
//   - disabled:   是否禁用整个输入（无会话时由外层控制）
//   - streaming:  是否正在流式生成
//
// emits:
//   - send(content: string)  点击发送 / Enter 提交时
//   - stop()                点击停止按钮时

import { computed, nextTick, ref, watch } from "vue";
import { SendHorizontal, Square } from "lucide-vue-next";

const props = defineProps<{
  disabled: boolean;
  streaming: boolean;
}>();

const emit = defineEmits<{
  send: [content: string];
  stop: [];
}>();

/** 文本草稿 */
const draft = ref<string>("");

/** textarea DOM 引用 */
const textareaRef = ref<HTMLTextAreaElement | null>(null);

/** 单行高度（与 line-height 一致） */
const LINE_HEIGHT_PX = 22;

/** 最大行数（含 padding 折算的余量） */
const MAX_ROWS = 6;

/** textarea 上下 padding 之和 */
const VERTICAL_PADDING_PX = 18;

/** textarea 的 maxHeight（px），用于自动增高封顶 */
const maxHeightPx = LINE_HEIGHT_PX * MAX_ROWS + VERTICAL_PADDING_PX;

/** textarea 的高度（px），由行数动态计算 */
const heightPx = ref<number>(LINE_HEIGHT_PX + VERTICAL_PADDING_PX);

/**
 * 根据 draft 内容计算 textarea 的高度（向上增高，封顶为 maxHeightPx）。
 * 通过临时重置为单行高度再读取 scrollHeight 获取精确高度，避免来回抖动。
 */
function autosize(): void {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = `${LINE_HEIGHT_PX + VERTICAL_PADDING_PX}px`;
  const sh = el.scrollHeight;
  const h = Math.min(sh, maxHeightPx);
  el.style.height = `${h}px`;
  heightPx.value = h;
}

/** 监听 draft 变化触发 autosize */
watch(draft, () => {
  void nextTick(autosize);
});

/** 实际是否禁用输入（流中或外层禁用时） */
const inputDisabled = computed<boolean>(() => props.disabled || props.streaming);

/** 提交（Enter） */
function handleSend(): void {
  if (inputDisabled.value) return;
  const v = draft.value.trim();
  if (!v) return;
  emit("send", v);
  draft.value = "";
  void nextTick(autosize);
}

/** 键盘事件：Enter 发送；Shift/Ctrl/Meta + Enter 换行 */
function onKeydown(e: KeyboardEvent): void {
  if (e.key !== "Enter") return;
  // Shift+Enter / Ctrl+Enter / Cmd+Enter 走默认换行
  if (e.shiftKey || e.ctrlKey || e.metaKey) return;
  // 纯 Enter 触发发送
  e.preventDefault();
  handleSend();
}

/** 工具栏发送按钮点击 */
function onSendClick(): void {
  handleSend();
}

/** 工具栏停止按钮点击 */
function onStopClick(): void {
  emit("stop");
}

/** 发送按钮是否禁用（无内容或外层禁用） */
const sendDisabled = computed<boolean>(
  () => inputDisabled.value || draft.value.trim().length === 0,
);
</script>

<template>
  <div
    :class="['chat-input', { 'chat-input-disabled': disabled, 'chat-input-streaming': streaming }]"
  >
    <textarea
      ref="textareaRef"
      v-model="draft"
      class="textarea"
      :placeholder="streaming ? '生成中...' : '输入消息，回车发送，Shift+Enter 换行'"
      :disabled="inputDisabled"
      rows="1"
      :style="{ height: `${heightPx}px` }"
      :maxlength="20000"
      @keydown="onKeydown"
    />
    <div class="toolbar">
      <div class="toolbar-left">
        <span class="hint-text">
          <template v-if="streaming">生成中 · 可随时停止</template>
          <template v-else>Enter 发送 · Shift+Enter 换行</template>
        </span>
      </div>
      <div class="toolbar-right">
        <button
          v-if="streaming"
          class="btn btn-stop"
          type="button"
          title="停止生成"
          aria-label="停止生成"
          @click="onStopClick"
        >
          <Square :size="14" aria-hidden="true" />
          <span class="btn-label">停止</span>
        </button>
        <button
          v-else
          class="btn btn-send"
          type="button"
          :disabled="sendDisabled"
          title="发送（Enter）"
          aria-label="发送消息"
          @click="onSendClick"
        >
          <SendHorizontal :size="16" aria-hidden="true" />
          <span class="btn-label">发送</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-input {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--ip-color-border-default);
  background-color: var(--ip-color-bg-secondary);
  padding: 12px 16px;
  flex-shrink: 0;
  position: relative;
}

.textarea {
  display: block;
  width: 100%;
  resize: none;
  /* 由行数动态控制 height（JS autosize 写入） */
  border: 1px solid var(--ip-input-border);
  /* 兼容旧版；新设计 token 为 --ip-color-border-default */
  border-radius: var(--ip-input-radius);
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  font-family: inherit;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-text-body-lh);
  color: var(--ip-color-text-primary);
  /* 输入区背景：设计 token 为 --ip-color-bg-primary / --ip-color-bg-secondary */
  background: var(--ip-input-bg);
  outline: none;
  box-sizing: border-box;
  overflow-y: auto;
  transition:
    border-color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out),
    background-color var(--ip-duration-base) var(--ip-ease-out);
}

.textarea::placeholder {
  color: var(--ip-color-text-tertiary);
}

.textarea:hover:not(:disabled) {
  border-color: var(--ip-color-border-strong);
}

.textarea:focus {
  border-color: var(--ip-color-border-focus);
  /* 焦点发光环：使用设计 token 中的 --ip-shadow-focus */
  box-shadow: var(--ip-shadow-focus);
}

.textarea:disabled {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
}

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 10px;
  min-height: 32px;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hint-text {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  appearance: none;
  height: var(--ip-btn-h-sm);
  padding: 0 14px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  border-radius: var(--ip-btn-radius);
  border: 1px solid transparent;
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn:active:not(:disabled) {
  transform: translateY(0.5px);
}

.btn-label {
  line-height: 1;
}

.btn-send {
  background: var(--ip-primary-600);
  color: var(--ip-color-text-on-primary);
  border-color: var(--ip-primary-600);
}

.btn-send:hover:not(:disabled) {
  background: var(--ip-primary-700);
  border-color: var(--ip-primary-700);
}

.btn-send:disabled {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  border-color: var(--ip-color-bg-tertiary);
  cursor: not-allowed;
}

.btn-stop {
  /* 红色高亮：用 danger token */
  background: var(--ip-danger-base);
  color: var(--ip-color-text-on-danger);
  border-color: var(--ip-danger-base);
}

.btn-stop:hover {
  background: var(--ip-danger-hover);
  border-color: var(--ip-danger-hover);
}

.btn-stop:focus-visible {
  box-shadow: var(--ip-shadow-focus-danger);
}

.btn-stop:active {
  background: var(--ip-danger-active);
  border-color: var(--ip-danger-active);
}

/* 流式中整块输入区视觉弱化（仅边框淡化） */
.chat-input-streaming .textarea {
  border-color: var(--ip-color-border-default);
}

/* 暗色模式：
 * --ip-color-bg-* / --ip-color-text-* / --ip-color-border-* 等系列 token
 * 均在 packages/ui/src/styles/tokens.css 的
 * @media (prefers-color-scheme: dark) 中自动重映射，
 * 因此本组件无需重新声明暗色覆盖块。
 */
</style>

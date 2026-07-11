<script setup lang="ts">
// 聊天输入框
//
// 职责：
//   - 多行 textarea，自动增高（最多 6 行高度）
//   - Enter 发送；Shift+Enter / Ctrl+Enter 换行
//   - 底部工具栏：左侧发送按钮（流中变为「停止」按钮 + 红色高亮）
//   - 流中禁用输入框与发送按钮（避免重复触发）
//
// props:
//   - disabled:   是否禁用整个输入（无会话时由外层控制）
//   - streaming:  是否正在流式生成
//
// emits:
//   - send(content: string)  点击发送 / Enter 提交时
//   - stop()                点击停止按钮时

import { computed, nextTick, ref, watch } from "vue";

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

/** 单行高度（用于计算最大高度） */
const LINE_HEIGHT_PX = 20;

/** 最大行数（含 padding 折算的余量） */
const MAX_ROWS = 6;

/** textarea 的 maxHeight（px），用于自动增高封顶 */
const maxHeightPx = LINE_HEIGHT_PX * MAX_ROWS + 20;

/** textarea 的 height（px），由行数动态计算 */
const heightPx = ref<number>(LINE_HEIGHT_PX + 20);

/**
 * 根据 draft 内容计算 textarea 的高度（向上增高，封顶为 maxHeightPx）。
 * 通过临时克隆再 scrollHeight 获取精确高度，避免来回抖动。
 */
function autosize(): void {
  const el = textareaRef.value;
  if (!el) return;
  // 先重置为单行高度以正确读取 scrollHeight
  el.style.height = `${LINE_HEIGHT_PX + 20}px`;
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

/** 发送按钮文案 */
const sendLabel = computed<string>(() => (props.streaming ? "发送中" : "发送"));

/** 提交（Enter） */
function handleSend(): void {
  if (inputDisabled.value) return;
  const v = draft.value.trim();
  if (!v) return;
  emit("send", v);
  draft.value = "";
  void nextTick(autosize);
}

/** 键盘事件 */
function onKeydown(e: KeyboardEvent): void {
  if (e.key !== "Enter") return;
  // Shift+Enter / Ctrl+Enter 走默认换行
  if (e.shiftKey || e.ctrlKey || e.metaKey) return;
  // 纯 Enter 触发发送
  e.preventDefault();
  handleSend();
}

/** 工具栏按钮点击（统一处理发送 / 停止） */
function onToolbarClick(): void {
  if (props.streaming) {
    emit("stop");
    return;
  }
  handleSend();
}
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
      :maxlength="20000"
      @keydown="onKeydown"
    />
    <div class="toolbar">
      <div class="toolbar-left">
        <span class="hint-text">
          <template v-if="streaming">生成中</template>
          <template v-else>Enter 发送 · Shift+Enter 换行</template>
        </span>
      </div>
      <div class="toolbar-right">
        <button
          v-if="streaming"
          class="btn btn-stop"
          type="button"
          title="停止生成"
          @click="onToolbarClick"
        >
          停止
        </button>
        <button
          v-else
          class="btn btn-send"
          type="button"
          :disabled="inputDisabled || draft.trim().length === 0"
          title="发送（Enter）"
          @click="onToolbarClick"
        >
          {{ sendLabel }}
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-input {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--border, #e0e0e0);
  background: var(--input-panel-bg, #ffffff);
  padding: 12px 16px 12px;
  flex-shrink: 0;
}

.textarea {
  display: block;
  width: 100%;
  resize: none;
  border: 1px solid var(--input-border, #d0d0d0);
  border-radius: 8px;
  padding: 10px 12px;
  font-family: inherit;
  font-size: 14px;
  line-height: 20px;
  color: var(--text-primary, #1a1a1a);
  background: var(--input-bg, #ffffff);
  outline: none;
  box-sizing: border-box;
  overflow-y: auto;
  transition: border-color 100ms ease;
}

.textarea:focus {
  border-color: var(--focus-border, #1a73e8);
}

.textarea:disabled {
  background: var(--input-bg-disabled, #f5f5f5);
  color: var(--text-secondary, #888);
  cursor: not-allowed;
}

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 8px;
  min-height: 28px;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hint-text {
  font-size: 11px;
  color: var(--text-secondary, #888);
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.btn {
  appearance: none;
  padding: 6px 18px;
  font-size: 13px;
  font-weight: 500;
  font-family: inherit;
  border-radius: 6px;
  border: 1px solid transparent;
  cursor: pointer;
  transition:
    background 100ms ease,
    color 100ms ease,
    border-color 100ms ease;
}

.btn-send {
  background: var(--accent-bg, #1a73e8);
  color: #fff;
}

.btn-send:hover:not(:disabled) {
  background: var(--accent-bg-hover, #1557b0);
}

.btn-send:disabled {
  background: var(--btn-disabled-bg, #cccccc);
  cursor: not-allowed;
}

.btn-stop {
  background: var(--danger-bg, #d93025);
  color: #fff;
  border-color: var(--danger-border, #d93025);
}

.btn-stop:hover {
  background: var(--danger-bg-hover, #b52a1f);
  border-color: var(--danger-border-hover, #b52a1f);
}

/* 流式中整块输入区视觉弱化 */
.chat-input-streaming .textarea {
  border-color: var(--input-border-streaming, #d0d0d0);
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .chat-input {
    --border: #3a3a4a;
    --input-panel-bg: #1e1e2e;
  }
  .textarea {
    --input-border: #4a4a5a;
    --input-bg: #2a2a3a;
    --text-primary: #f0f0f0;
    --focus-border: #4a90e2;
  }
  .textarea:disabled {
    --input-bg-disabled: #252535;
    --text-secondary: #777;
  }
  .hint-text {
    --text-secondary: #888;
  }
  .btn-send:disabled {
    --btn-disabled-bg: #3a3a4a;
  }
  .btn-stop {
    --danger-bg: #c0392b;
    --danger-border: #c0392b;
  }
  .btn-stop:hover {
    --danger-bg-hover: #a93226;
    --danger-border-hover: #a93226;
  }
}
</style>

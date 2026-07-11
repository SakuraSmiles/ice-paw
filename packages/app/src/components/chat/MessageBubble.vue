<script setup lang="ts">
// 单条消息气泡
//
// 职责：
//   - 按消息 role 渲染不同样式：用户（右）、助手（左）、系统（居中灰字）
//   - 流式中的助手消息尾部追加光标闪烁动画
//   - 错误态：红色边框 + 错误文本 + 「重试」按钮（重试功能 P1，目前仅占位）
//
// props:
//   - message:     Message 实体
//   - isStreaming: 是否为正在流式生成的那条（用于显示光标）
//
// emits:
//   - retry: 点击重试按钮时触发

import { computed } from "vue";
import type { Message } from "../../types";

const props = defineProps<{
  message: Message;
  isStreaming: boolean;
}>();

const emit = defineEmits<{
  retry: [message: Message];
}>();

/** 是否展示光标（仅流式中且无错误） */
const showCursor = computed<boolean>(
  () => props.isStreaming && !props.message.error && props.message.role === "assistant",
);

/** 重试按钮点击 */
function onRetry(): void {
  emit("retry", props.message);
}
</script>

<template>
  <div :class="['bubble-row', `bubble-row-${message.role}`]">
    <!-- 系统消息：居中灰字 -->
    <div v-if="message.role === 'system'" class="bubble-system">
      <span class="bubble-system-text">{{ message.content }}</span>
    </div>

    <!-- 用户消息：右侧气泡 -->
    <div v-else-if="message.role === 'user'" class="bubble-user">
      <div class="bubble-content">
        <div class="bubble-text">{{ message.content }}</div>
        <div v-if="message.error" class="bubble-error">
          <span class="error-text">{{ message.error }}</span>
        </div>
      </div>
    </div>

    <!-- 助手消息：左侧气泡 -->
    <div v-else class="bubble-assistant">
      <div class="bubble-content">
        <div class="bubble-text">
          <span class="bubble-text-content">{{ message.content }}</span>
          <span v-if="showCursor" class="bubble-cursor" aria-hidden="true" />
        </div>
        <div v-if="message.error" class="bubble-error">
          <span class="error-text">{{ message.error }}</span>
          <button class="btn-retry" type="button" @click="onRetry">重试</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.bubble-row {
  display: flex;
  width: 100%;
  margin: 8px 0;
  padding: 0 20px;
  box-sizing: border-box;
}

/* 用户消息：右对齐 */
.bubble-row-user {
  justify-content: flex-end;
}

/* 助手消息：左对齐 */
.bubble-row-assistant {
  justify-content: flex-start;
}

/* 系统消息：居中 */
.bubble-row-system {
  justify-content: center;
}

.bubble-content {
  max-width: min(720px, 80%);
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.bubble-text {
  padding: 10px 14px;
  border-radius: 10px;
  font-size: 14px;
  line-height: 1.55;
  white-space: pre-wrap;
  word-break: break-word;
  overflow-wrap: anywhere;
  position: relative;
}

.bubble-text-content {
  white-space: pre-wrap;
  word-break: break-word;
}

/* 用户气泡：深色背景 */
.bubble-user .bubble-text {
  background: var(--user-bubble-bg, #1a73e8);
  color: var(--user-bubble-fg, #ffffff);
  border-bottom-right-radius: 4px;
}

/* 助手气泡：浅色 / 透明背景 */
.bubble-assistant .bubble-text {
  background: var(--assistant-bubble-bg, rgba(0, 0, 0, 0.04));
  color: var(--text-primary, #1a1a1a);
  border-bottom-left-radius: 4px;
}

/* 系统气泡 */
.bubble-system {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px 12px;
}

.bubble-system-text {
  font-size: 12px;
  color: var(--text-secondary, #888);
  background: var(--system-bg, rgba(0, 0, 0, 0.04));
  padding: 4px 12px;
  border-radius: 10px;
}

/* 流式光标 */
.bubble-cursor {
  display: inline-block;
  width: 7px;
  height: 14px;
  margin-left: 2px;
  vertical-align: text-bottom;
  background: var(--cursor-fg, currentColor);
  animation: cursor-blink 1s steps(1) infinite;
  border-radius: 1px;
}

@keyframes cursor-blink {
  0%,
  50% {
    opacity: 1;
  }
  50.01%,
  100% {
    opacity: 0;
  }
}

/* 错误条 */
.bubble-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 12px;
  font-size: 12px;
  border: 1px solid var(--danger-border, #d93025);
  border-radius: 6px;
  background: var(--danger-bg-light, #fde8e8);
  color: var(--danger-fg, #d93025);
}

.error-text {
  flex: 1;
  word-break: break-word;
}

.btn-retry {
  flex-shrink: 0;
  padding: 2px 10px;
  font-size: 12px;
  border: 1px solid var(--danger-border, #d93025);
  border-radius: 4px;
  background: transparent;
  color: var(--danger-fg, #d93025);
  cursor: pointer;
  font-family: inherit;
  transition: background 100ms ease;
}

.btn-retry:hover {
  background: var(--danger-bg-hover, #fcd9d9);
}

/* 暗色模式 */
@media (prefers-color-scheme: dark) {
  .bubble-assistant .bubble-text {
    --assistant-bubble-bg: rgba(255, 255, 255, 0.06);
    --text-primary: #f0f0f0;
  }
  .bubble-user .bubble-text {
    --user-bubble-bg: #1a73e8;
    --user-bubble-fg: #ffffff;
  }
  .bubble-system-text {
    --text-secondary: #888;
    --system-bg: rgba(255, 255, 255, 0.06);
  }
  .bubble-error {
    --danger-border: #c62828;
    --danger-bg-light: #3a2020;
    --danger-fg: #ff8a80;
  }
  .btn-retry {
    --danger-border: #c62828;
    --danger-fg: #ff8a80;
    --danger-bg-hover: #4a2828;
  }
}
</style>

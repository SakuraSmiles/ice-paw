<script setup lang="ts">
// 单条消息气泡
//
// 职责：
//   - 按消息 role 渲染不同样式：用户（右）、助手（左）、系统（居中灰字）
//   - 助手消息正文用 MarkdownContent 渲染（标题 / 粗体 / 代码块等）
//   - 用户消息保持纯文本 + white-space: pre-wrap（保留换行）
//   - 流式中的助手消息尾部追加光标闪烁动画（::after 伪元素）
//   - 错误态：红色边框 + 错误文本 + 「重试」按钮（仅助手；重试功能 P1，目前仅占位）
//
// props:
//   - message:     Message 实体
//   - isStreaming: 是否为正在流式生成的那条（用于显示光标）
//   - prevRole:    上一条消息的角色（用于跨 agent 间距；null 表示这是首条）
//
// emits:
//   - retry: 点击重试按钮时触发

import { computed } from "vue";
import type { Message, MessageRole } from "../../types";
import MarkdownContent from "./MarkdownContent.vue";

const props = defineProps<{
  message: Message;
  isStreaming: boolean;
  /** 上一条消息的角色；null 表示这是列表里的第一条 */
  prevRole?: MessageRole | null;
}>();

const emit = defineEmits<{
  retry: [message: Message];
}>();

/** 当前消息角色快捷判断 */
const isAssistant = computed<boolean>(() => props.message.role === "assistant");
const isUser = computed<boolean>(() => props.message.role === "user");

/** 是否展示光标（仅流式中的助手消息、无错误） */
const showCursor = computed<boolean>(
  () => props.isStreaming && !props.message.error && isAssistant.value,
);

/** 是否为跨 agent（上一条存在且角色不同） */
const isCrossAgent = computed<boolean>(
  () => props.prevRole != null && props.prevRole !== props.message.role,
);

/**
 * 顶部间距：跨 agent 用更大间距，连续同 agent 用更紧凑间距。
 * token 来自 @ice-paw/ui 的 design system；当前主应用未引入 tokens.css，
 * 因此提供合理 fallback。
 */
const marginTop = computed<string>(() =>
  isCrossAgent.value
    ? "var(--ip-message-gap-cross, 16px)"
    : "var(--ip-message-gap-same, 4px)",
);

/** 重试按钮点击 */
function onRetry(): void {
  emit("retry", props.message);
}
</script>

<template>
  <div
    :class="['bubble-row', `bubble-row-${message.role}`]"
    :style="{ marginTop }"
  >
    <!-- 系统消息：居中灰字 -->
    <div v-if="message.role === 'system'" class="bubble-system">
      <span class="bubble-system-text">{{ message.content }}</span>
    </div>

    <!-- 用户消息：右侧气泡，纯文本 -->
    <div v-else-if="isUser" class="bubble-user">
      <div class="bubble-content">
        <div class="bubble-text">
          <div class="bubble-text-content">{{ message.content }}</div>
        </div>
        <div v-if="message.error" class="bubble-error">
          <span class="error-text">{{ message.error }}</span>
        </div>
      </div>
    </div>

    <!-- 助手消息：左侧气泡，Markdown 渲染 -->
    <div v-else class="bubble-assistant">
      <div class="bubble-content">
        <div
          class="bubble-text"
          :class="{ 'bubble-text-streaming': showCursor }"
        >
          <MarkdownContent :content="message.content" />
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
  max-width: var(--ip-message-max-w, min(720px, 80%));
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
  word-break: break-word;
  overflow-wrap: anywhere;
  position: relative;
  /* flex column 让流式光标（::after 伪元素）自然落在 markdown 末尾换行处 */
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.bubble-text-content {
  /* 用户消息：保留原始换行与空格 */
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

/* 流式光标：仅助手消息、正在流式生成、且无错误时出现
   用 ::after 伪元素附加在 bubble-text 末尾，不污染 markdown 渲染结果 */
.bubble-text-streaming::after {
  content: "";
  display: inline-block;
  width: 7px;
  height: 14px;
  margin-top: 2px;
  align-self: flex-start;
  background: var(--cursor-fg, currentColor);
  animation: cursor-blink 1s steps(1) infinite;
  border-radius: 1px;
  /* a11y: 屏幕阅读器忽略装饰性光标 */
  speak: none;
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

/* 减少动效偏好：光标常亮不闪烁，避免对前庭敏感用户造成干扰 */
@media (prefers-reduced-motion: reduce) {
  .bubble-text-streaming::after {
    animation: none;
    opacity: 1;
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

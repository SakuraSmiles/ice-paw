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
//   - message:        Message 实体
//   - isStreaming:    是否为正在流式生成的那条（用于显示光标）
//   - renderMarkdown: 是否渲染 Markdown。流式中传 false 走纯文本路径以减轻性能压力；
//                     流式结束后由父组件切到 true，触发完整 Markdown 渲染。
//   - prevRole:       上一条消息的角色（用于跨 agent 间距；null 表示这是首条）
//
// emits:
//   - retry: 点击重试按钮时触发

import { computed } from "vue";
import type { Message, MessageRole } from "../../types";
import MarkdownContent from "./MarkdownContent.vue";

const props = defineProps<{
  message: Message;
  isStreaming: boolean;
  /**
   * 是否渲染 Markdown。
   * - 流式中（message.id === streamingId）传 false：直接展示原文 + 光标，
   *   跳过 markdown-it 解析 + highlight.js 高亮，避免每个 chunk 都跑一次完整渲染。
   * - 流式结束后传 true：切到 MarkdownContent 渲染完整 Markdown。
   */
  renderMarkdown: boolean;
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
  <!--
    列表级 v-memo（作用在外层 bubble-row）：
      - 依赖项：message.content / message.role / renderMarkdown。
      - 历史消息稳定 → deps 不变 → Vue 跳过整个子树 patch，
        避免列表被 stream 增量刷新时把整列历史气泡也连带 patch 一次。
      - 流式中：deps 每 chunk 都变（content 递增），v-memo 正确触发更新，
        不会卡死。配合 MessageList 流式期间不传 renderMarkdown=true，
        单 chunk 成本仅 1 个文本节点 + v-memo 短路检查。
  -->
  <div
    v-memo="[message.content, message.role, renderMarkdown]"
    :class="['bubble-row', `bubble-row-${message.role}`]"
    :style="{ marginTop }"
  >
    <!-- 系统消息：居中灰字 -->
    <div v-if="message.role === 'system'" class="bubble-system">
      <span class="bubble-system-text">{{ message.content }}</span>
    </div>

    <!-- 用户消息：右侧气泡，纯文本（保持原始换行与空格） -->
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

    <!--
      助手消息：左侧气泡。
      流式渲染策略：
        - 流式中（renderMarkdown=false）→ 直接渲染纯文本 + 光标，跳过 markdown-it
        - 流式结束（renderMarkdown=true） → 切到 MarkdownContent 完整渲染
      切换由父组件控制：流式开始传 false，每 chunk 期间保持 false；
      流式完成（chat:done）那一刻翻转成 true，触发 MarkdownContent 接管。
    -->
    <div v-else class="bubble-assistant">
      <div class="bubble-content">
        <div
          class="bubble-text"
          :class="{ 'bubble-text-streaming': showCursor }"
        >
          <MarkdownContent
            v-if="renderMarkdown"
            :content="message.content"
          />
          <div v-else class="bubble-text-content">{{ message.content }}</div>
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
  background: var(--ip-color-bg-user-bubble);
  color: var(--ip-color-text-on-primary);
  border-bottom-right-radius: 4px;
}

/* 助手气泡：浅色 / 透明背景 */
.bubble-assistant .bubble-text {
  background: var(--ip-color-bg-ai-bubble);
  color: var(--ip-color-text-primary);
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
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-tertiary);
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
  font-size: var(--ip-text-caption-size);
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md);
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}

.error-text {
  flex: 1;
  word-break: break-word;
}

.btn-retry {
  flex-shrink: 0;
  padding: 2px 10px;
  font-size: var(--ip-text-caption-size);
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-danger-text);
  cursor: pointer;
  font-family: inherit;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-retry:hover {
  background: var(--ip-danger-bg);
}
</style>

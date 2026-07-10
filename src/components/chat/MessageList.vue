<script setup lang="ts">
// 消息列表
//
// 职责：
//   - 渲染消息列表（v-for MessageBubble）
//   - 新消息 / 流式增量时自动滚动到底部
//   - 用户向上滚动后不强制回滚到底（让用户继续阅读历史）
//
// props:
//   - messages:    Message[]   当前会话的消息（含 store 流式占位）
//   - streamingId: string|null 正在流式生成的助手消息 ID（用于光标显示）
//
// emits:
//   - retry(message: Message)  用户点击重试按钮

import { nextTick, ref, watch } from "vue";
import type { Message } from "../../types";
import MessageBubble from "./MessageBubble.vue";

const props = defineProps<{
  messages: Message[];
  streamingId: string | null;
}>();

const emit = defineEmits<{
  retry: [message: Message];
}>();

/** 列表容器 DOM 引用 */
const listRef = ref<HTMLDivElement | null>(null);

/** 用户是否在底部附近（距底部 ≤ 80px 视为「在底部」） */
const pinnedToBottom = ref<boolean>(true);

/**
 * 滚动到底部（仅在 pinnedToBottom=true 时调用）。
 * 使用 nextTick 等 DOM 更新完成。
 */
async function scrollToBottom(): Promise<void> {
  await nextTick();
  const el = listRef.value;
  if (!el) return;
  el.scrollTop = el.scrollHeight;
}

/**
 * 监听用户手动滚动，更新 pinnedToBottom。
 * 当滚动高度 + 视口高度 >= 滚动总高度 - 80 时认为在底部。
 */
function onScroll(): void {
  const el = listRef.value;
  if (!el) return;
  const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
  pinnedToBottom.value = distanceFromBottom <= 80;
}

/**
 * 监听 messages 变化：若 pinnedToBottom 则滚到底。
 * 流式中 streamingContent 增长时通过 reactive 触发本 watcher。
 */
watch(
  () => props.messages,
  () => {
    if (pinnedToBottom.value) {
      void scrollToBottom();
    }
  },
  { deep: true },
);

/** 重试回调透传 */
function onRetry(msg: Message): void {
  emit("retry", msg);
}
</script>

<template>
  <div ref="listRef" class="message-list" @scroll="onScroll">
    <div class="message-list-inner">
      <MessageBubble
        v-for="msg in messages"
        :key="msg.id"
        :message="msg"
        :is-streaming="msg.id === streamingId"
        @retry="onRetry"
      />
    </div>
  </div>
</template>

<style scoped>
.message-list {
  flex: 1 1 auto;
  min-height: 0;
  overflow-y: auto;
  background: var(--chat-bg, #fafafa);
  scroll-behavior: smooth;
}

.message-list-inner {
  display: flex;
  flex-direction: column;
  padding: 16px 0 24px;
  min-height: 100%;
  box-sizing: border-box;
}

/* 滚动条暗色适配 */
@media (prefers-color-scheme: dark) {
  .message-list {
    --chat-bg: #181828;
  }
}
</style>

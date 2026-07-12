<script setup lang="ts">
// 消息列表
//
// 职责：
//   - 渲染消息列表（v-for MessageBubble）
//   - 新消息 / 流式增量时自动滚动到底部
//   - 用户向上滚动后不强制回滚到底（让用户继续阅读历史）
//   - 控制子组件的 renderMarkdown：streaming 中传 false 走纯文本路径，
//     流式结束后翻为 true 切到 MarkdownContent
//
// props:
//   - messages:    Message[]   当前会话的消息（含 store 流式占位）
//   - streamingId: string|null 正在流式生成的助手消息 ID（用于光标显示）
//
// emits:
//   - retry(message: Message)  用户点击重试按钮

import { ref, watch } from "vue";
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
 * 记录「上一次参与 v-memo 判断」的快照，用于决定下一帧 Markdown 开关。
 * - 列表长度增加  → 新消息到达，先用纯文本占位
 * - 列表长度不变 + 末条 content 仍变 → 仍在流式中，保持纯文本
 * - 列表长度 / content 都不再变     → 认为流式结束，翻成 Markdown
 */
const renderMarkdown = ref<Record<string, boolean>>({});

/**
 * 滚动到底部（仅在 pinnedToBottom=true 时调用）。
 * 改用 requestAnimationFrame：与浏览器绘制同步，比 nextTick 更节流
 * （nextTick 是 microtask，可能在流式高频 chunk 下反复入队）。
 * rAF 在每帧最多触发 1 次，与显示器刷新率对齐，体感更顺滑。
 */
function scrollToBottom(): void {
  const el = listRef.value;
  if (!el) return;
  requestAnimationFrame(() => {
    const target = listRef.value;
    if (target) target.scrollTop = target.scrollHeight;
  });
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
 * 浅层 watch：监听列表长度 + 末条 content 长度。
 * 相比 deep:true，流式 chunk 期间不再递归遍历整个 messages 数组，
 * 只看引用/length 和末条 content 的 string length，watcher 开销几乎为 0。
 *
 * 副作用：
 *   1. 维持底部：若用户在底部附近，则滚到底。
 *   2. 维护 renderMarkdown 映射：消息数变化 / 末条变化 → 保持 false；
 *      内容稳态（连续两帧 length 不变 + 末条 content 长度不变）
 *      → 流式结束，翻为 true。
 */
watch(
  () => {
    const list = props.messages;
    const last = list.length > 0 ? list[list.length - 1] : null;
    return {
      length: list.length,
      // 只看末条 content 的长度，避免深递归 + 减少比较成本
      lastContentLen: last ? last.content.length : 0,
      lastId: last?.id ?? null,
      streamingId: props.streamingId,
    };
  },
  (next, prev) => {
    if (pinnedToBottom.value) {
      scrollToBottom();
    }

    const map = { ...renderMarkdown.value };
    let touched = false;

    // 流式中的助手消息：保持 false（纯文本 + 光标）
    if (props.streamingId && map[props.streamingId] !== false) {
      map[props.streamingId] = false;
      touched = true;
    }

    // 对列表里每条不是「正在流式」的助手消息，确保是 Markdown 渲染
    for (const msg of props.messages) {
      if (msg.role !== "assistant") continue;
      if (msg.id === props.streamingId) continue;
      if (map[msg.id] !== true) {
        map[msg.id] = true;
        touched = true;
      }
    }

    // 流式结束判定（在 chat:done / chat:error 等事件之后）：
    //   a) 主流路径：streamingId 由非 null 翻为 null（chatStore.isStreaming 关闭）。
    //      把上一帧还在流式的那条消息标记为 Markdown。
    //   b) 兜底路径：streamingId 仍为非 null（事件延迟），但末条 content 长度
    //      连续两帧不变，且 lastId === streamingId；说明内容已停止增长，
    //      也视为流式结束。
    const prevStreaming = prev?.streamingId ?? null;
    if (prevStreaming != null && next.streamingId == null) {
      if (map[prevStreaming] !== true) {
        map[prevStreaming] = true;
        touched = true;
      }
    } else if (
      prev != null &&
      next.streamingId != null &&
      next.lastId === next.streamingId &&
      prev.lastContentLen === next.lastContentLen &&
      next.lastContentLen > 0
    ) {
      if (map[next.streamingId] !== true) {
        map[next.streamingId] = true;
        touched = true;
      }
    }

    if (touched) renderMarkdown.value = map;
  },
);

/** 重试回调透传 */
function onRetry(msg: Message): void {
  emit("retry", msg);
}

/**
 * 判断某条消息在当前帧是否应渲染 Markdown。
 * 未在映射中 → 保守默认：非流式 → true（历史的助手消息默认走 Markdown）
 */
function shouldRenderMarkdown(msg: Message): boolean {
  if (msg.role !== "assistant") return false; // 用户 / 系统消息永远不渲染 Markdown
  const inMap = renderMarkdown.value[msg.id];
  if (inMap !== undefined) return inMap;
  // 没在映射里：非流式的历史助手消息默认 Markdown；正在流式则 false
  return msg.id !== props.streamingId;
}
</script>

<template>
  <div ref="listRef" class="message-list" @scroll="onScroll">
    <div class="message-list-inner">
      <MessageBubble
        v-for="(msg, idx) in messages"
        :key="msg.id"
        :message="msg"
        :is-streaming="msg.id === streamingId"
        :render-markdown="shouldRenderMarkdown(msg)"
        :prev-role="idx > 0 ? messages[idx - 1]!.role : null"
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
  background: var(--ip-color-bg-secondary);
  scroll-behavior: smooth;
}

.message-list-inner {
  display: flex;
  flex-direction: column;
  padding: 16px 0 24px;
  min-height: 100%;
  box-sizing: border-box;
}
</style>

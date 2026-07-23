<script setup lang="ts">
// 消息列表
//
// 职责：
//   - 渲染消息列表（v-for MessageBubble）
//   - 新消息 / 流式增量时自动滚动到底部
//   - 用户向上滚动后不强制回滚到底（让用户继续阅读历史）
//   - 控制子组件的 renderMarkdown：streaming 中传 false 走纯文本路径，
//     流式结束后翻为 true 切到 MarkdownContent
//   - **P2 性能优化**：向上滚动懒加载历史消息，加载期间保持滚动位置不跳动
//
// props:
//   - messages:       Message[]   当前会话的消息（含 store 流式占位）
//   - streamingId:    string|null 正在流式生成的助手消息 ID（用于光标显示）
//   - hasMoreOlder:   boolean     是否还有更早历史可加载（向上翻页用）
//   - loadingOlder:   boolean     向上翻页 in-flight 标志
//   - loading:        boolean     初始加载 in-flight（用于显示骨架 / 空态提示）
//
// emits:
//   - retry(message: Message)         用户点击重试按钮
//   - load-older()                    用户滚到顶部附近，请求加载更多历史
//
// expose:
//   - forceBottom()                   命令式方法：强制滚动到底部
//                                     （切换会话 / 初始加载完成后由父组件调用）

import { nextTick, reactive, ref, watch } from "vue";
import type { Message } from "../../types";
import MessageBubble from "./MessageBubble.vue";
import { useChatStore } from "../../stores/chat";

/** P2-3: Token usage from last completed stream */
const chatStore = useChatStore();

/** 实时工具调用类型（与 store 中定义一致） */
interface ActiveToolCall {
  id: string;
  name: string;
  argumentsBuffer: string;
  ended: boolean;
}

const props = defineProps<{
  messages: Message[];
  streamingId: string | null;
  isRetrying?: boolean;
  retryProgress?: string;
  /** P2-1: 实时活跃的工具调用 */
  activeToolCalls?: ActiveToolCall[];
  /** P2-1: 实时思考过程内容 */
  thinkingContent?: string;
  /** P2: 是否还有更早历史可加载 */
  hasMoreOlder?: boolean;
  /** P2: 向上翻页 in-flight 标志 */
  loadingOlder?: boolean;
  /** P3: 初始加载 in-flight（用于空态骨架屏） */
  loading?: boolean;
}>();

const emit = defineEmits<{
  retry: [message: Message];
  /** P2: 用户滚到顶部附近，请求加载更多历史 */
  "load-older": [];
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

// ============================================================================
// P2 性能优化：滚动位置补偿（向上翻页时不让视口跳动）
// ============================================================================

/** 用户滚到距顶部多少 px 内即触发向上翻页 */
const TOP_THRESHOLD = 80;

/**
 * 滚动锚点：记录加载历史前的 scrollHeight + scrollTop，
 * 加载完成后用「new scrollHeight - old scrollHeight」补偿 scrollTop。
 *
 * scrollHeight = 0 表示无未决锚点（避免误判）。
 */
const scrollAnchor = reactive({
  scrollHeight: 0,
  scrollTop: 0,
});

/**
 * 滚动到底部（仅在 pinnedToBottom=true 时由 watch 调用）。
 * 改用 requestAnimationFrame：与浏览器绘制同步，比 nextTick 更节流
 * （nextTick 是 microtask，可能在流式高频 chunk 下反复入队）。
 * rAF 在每帧最多触发 1 次，与显示器刷新率对齐，体感更顺滑。
 *
 * 暴露给外部的 forceBottom() 也走同一套路径。
 */
function scrollToBottom(): void {
  const el = listRef.value;
  if (!el) return;
  requestAnimationFrame(() => {
    const target = listRef.value;
    if (!target) return;
    // 临时关闭 smooth scrolling，确保立即跳到底部
    target.style.scrollBehavior = "auto";
    target.scrollTop = target.scrollHeight;
    // 恢复 smooth 以便用户手动滚动时平滑
    requestAnimationFrame(() => {
      target.style.scrollBehavior = "";
    });
  });
}

/**
 * 监听用户手动滚动，更新 pinnedToBottom + 检测顶部触发向上翻页。
 *
 * 底部判定：scrollHeight - scrollTop - clientHeight <= 80
 * 顶部触发：scrollTop <= TOP_THRESHOLD 且 hasMoreOlder 且 非加载中 且 列表非空
 *          → 记录 scrollAnchor（加载前的 scrollHeight / scrollTop）
 *          → emit("load-older")，由父组件调 store.loadOlderMessages()
 */
function onScroll(): void {
  const el = listRef.value;
  if (!el) return;

  // 底部检测（保持原有逻辑）
  const distanceFromBottom = el.scrollHeight - el.scrollTop - el.clientHeight;
  pinnedToBottom.value = distanceFromBottom <= 80;

  // 顶部检测：触发加载更多
  if (
    el.scrollTop <= TOP_THRESHOLD &&
    props.hasMoreOlder &&
    !props.loadingOlder &&
    props.messages.length > 0
  ) {
    // 记录锚点：wait for the DOM update after messages prepend,
    // then compensate via the messages.length watcher below.
    scrollAnchor.scrollHeight = el.scrollHeight;
    scrollAnchor.scrollTop = el.scrollTop;
    emit("load-older");
  }
}

/**
 * watch messages 长度 + 末条 content 长度。
 *
 * 三件事一起处理：
 *   1. 维持底部：若用户在底部附近，则滚到底。
 *   2. 滚动补偿：若长度增加且有 anchor，说明是「加载更多历史」导致的 prepend，
 *      用 new scrollHeight - old scrollHeight 补偿 scrollTop。
 *   3. anchor 重置：若长度减少（典型：会话切换后清空再加载新数据），
 *      显式重置 anchor（避免下次 prepend 误用陈旧锚点）。
 *
 * 流式结束判定（renderMarkdown 翻 true）逻辑保持不变。
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
    // ---------- 1. 维持底部 ----------
    if (pinnedToBottom.value) {
      scrollToBottom();
    }

    // ---------- 2. 滚动补偿（向上翻页 prepend） ----------
    if (next.length > (prev?.length ?? 0) && scrollAnchor.scrollHeight > 0) {
      nextTick(() => {
        const el = listRef.value;
        if (!el) return;
        const diff = el.scrollHeight - scrollAnchor.scrollHeight;
        // 用户滚动条位置保持不变，但视口内容向下平移了 diff 像素，
        // 让用户看到的是「原本那条消息仍然在原位置，新增的历史在其上方」。
        el.scrollTop = scrollAnchor.scrollTop + diff;
        // 消费完锚点，避免下次误用
        scrollAnchor.scrollHeight = 0;
        scrollAnchor.scrollTop = 0;
      });
    } else if (next.length < (prev?.length ?? 0)) {
      // ---------- 3. anchor 重置（会话切换 / 数据减少） ----------
      scrollAnchor.scrollHeight = 0;
      scrollAnchor.scrollTop = 0;
    }

    // ---------- 流式结束判定（renderMarkdown 翻转） ----------
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

// ============================================================================
// expose：父组件命令式接口
// ============================================================================

/**
 * 命令式方法：强制滚动到底部（不等 watch 的 pinnedToBottom 反应）。
 *
 * 使用场景：
 *   - 切换会话后，store.loadMessages() 解析完毕 → 调用此方法
 *   - 冷启动初始加载完成 → 调用此方法
 *   - 用户主动滚到顶部触发翻页 → 不调用（翻页由滚动锚点补偿，保持视口）
 *
 * 实现：nextTick 等 DOM 更新完 → scrollToBottom。
 * 该方法连同下面的空态提示一起，确保用户体验：
 *   「切换会话后，从底部看新会话的最新消息，不会先看到顶部再滚下」。
 */
function forceBottom(): void {
  // 先重置 pinnedToBottom 确保下一次 watch 也会滚到底
  pinnedToBottom.value = true;
  nextTick(() => {
    scrollToBottom();
    // 二次 nextTick：兜底首屏 v-for 全部 patch 完成
    nextTick(() => scrollToBottom());
  });
}

defineExpose({
  forceBottom,
});
</script>

<template>
  <div ref="listRef" class="message-list" @scroll="onScroll">
    <!--
      P2 加载指示器区：
        - loading-older → spinner + "加载历史消息..."
        - 否则如果 hasMoreOlder && messages.length > 0 → "向上滚动加载更多"
        - 否则如果 !hasMoreOlder && messages.length > 0 → "已经到顶了"（P3 终止提示）
        - 否则（数据极少 / 初次加载） → 不显示
    -->
    <div v-if="loadingOlder" class="load-indicator load-indicator-loading">
      <span class="load-spinner" aria-hidden="true" />
      <span class="load-text">加载历史消息...</span>
    </div>
    <div
      v-else-if="hasMoreOlder && messages.length > 0"
      class="load-indicator load-indicator-hint"
    >
      <span class="load-text">向上滚动加载更多</span>
    </div>
    <div
      v-else-if="!hasMoreOlder && messages.length > 0"
      class="load-indicator load-indicator-end"
    >
      <span class="load-text">没有更多历史消息了</span>
    </div>

    <div class="message-list-inner">
      <MessageBubble
        v-for="(msg, idx) in messages"
        :key="msg.id"
        v-memo="[
          msg.content,
          msg.error,
          msg.content_blocks,
          streamingId === msg.id,
          idx === messages.length - 1,
          idx === messages.length - 1 ? chatStore.lastUsage : null,
          msg.model,
        ]"
        :message="msg"
        :is-streaming="msg.id === streamingId"
        :render-markdown="shouldRenderMarkdown(msg)"
        :prev-role="idx > 0 ? messages[idx - 1]!.role : null"
        :is-retrying="isRetrying && msg.id === streamingId"
        :retry-progress="retryProgress"
        :active-tool-calls="msg.id === streamingId ? activeToolCalls : []"
        :thinking-content="msg.id === streamingId ? thinkingContent : ''"
        :usage="idx === messages.length - 1 && msg.role === 'assistant' ? chatStore.lastUsage : null"
        @retry="onRetry"
      />
    </div>

    <!--
      P3 体验：初始加载骨架屏 / 占位。
      - 仅当 loading=true 且 messages 为空时显示，避免与「有消息但正在流式」混淆
      - 切换会话时短暂可见，加载完即消失
    -->
    <div
      v-if="loading && messages.length === 0"
      class="message-list-skeleton"
      role="status"
      aria-label="加载中"
    >
      <div class="skeleton-bubble skeleton-bubble-user">
        <div class="skeleton-line skeleton-line-short" />
      </div>
      <div class="skeleton-bubble skeleton-bubble-assistant">
        <div class="skeleton-line skeleton-line-long" />
        <div class="skeleton-line skeleton-line-medium" />
      </div>
      <div class="skeleton-bubble skeleton-bubble-assistant">
        <div class="skeleton-line skeleton-line-medium" />
      </div>
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

/* ============================================================================
 * 顶部加载指示器
 * ============================================================================ */

.load-indicator {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--ip-spacing-2, 8px);
  padding: var(--ip-spacing-2, 8px) var(--ip-spacing-3, 12px);
  font-size: var(--ip-text-caption-size, 12px);
  line-height: 1.4;
  color: var(--ip-color-text-tertiary);
  -webkit-user-select: none;
  user-select: none;
}

/* 提示文字（淡灰、居中） */
.load-text {
  font-weight: var(--ip-font-weight-regular, 400);
}

/* 旋转 spinner：复用 UI 库的 ip-spin 关键帧（已全局注册） */
.load-spinner {
  display: inline-block;
  width: 12px;
  height: 12px;
  border: 2px solid var(--ip-color-border-default, var(--ip-gray-300, #d9d9d9));
  border-top-color: var(--ip-color-text-tertiary, var(--ip-gray-500, #8c8c8c));
  border-radius: 50%;
  animation: ip-spin var(--ip-duration-spinner, 720ms) linear infinite;
  flex-shrink: 0;
}

/* 加载中态：稍大 padding，给用户视觉缓冲 */
.load-indicator-loading {
  padding-top: var(--ip-spacing-3, 12px);
  padding-bottom: var(--ip-spacing-3, 12px);
}

/* 终止提示：「没有更多历史消息了」用更淡的颜色 + 顶部细线分隔 */
.load-indicator-end {
  padding-top: var(--ip-spacing-4, 16px);
  color: var(--ip-color-text-disabled, var(--ip-gray-400, #bfbfbf));
}

/* 减少动效偏好：去掉 spinner 动画 */
@media (prefers-reduced-motion: reduce) {
  .load-spinner {
    animation: none;
    border-top-color: var(--ip-color-border-default, var(--ip-gray-300, #d9d9d9));
  }
}

/* ============================================================================
 * P3 初始加载骨架屏
 * ============================================================================ */

.message-list-skeleton {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-4, 16px);
  padding: var(--ip-spacing-6, 24px) var(--ip-spacing-5, 20px);
  min-height: 200px;
}

.skeleton-bubble {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2, 8px);
  max-width: var(--ip-message-max-w, 720px);
}

.skeleton-bubble-user {
  align-self: flex-end;
  align-items: flex-end;
}

.skeleton-bubble-assistant {
  align-self: flex-start;
}

/* 占位横条：浅灰底 + 闪烁动画 */
.skeleton-line {
  height: 14px;
  border-radius: var(--ip-radius-md, 6px);
  background: linear-gradient(
    90deg,
    var(--ip-color-bg-tertiary, var(--ip-gray-100, #f5f5f5)) 0%,
    var(--ip-color-bg-secondary, var(--ip-gray-200, #ebebeb)) 50%,
    var(--ip-color-bg-tertiary, var(--ip-gray-100, #f5f5f5)) 100%
  );
  background-size: 200% 100%;
  animation: ip-skeleton-shimmer var(--ip-duration-skeleton, 1800ms) ease-in-out infinite;
}

.skeleton-line-short {
  width: 35%;
}

.skeleton-line-medium {
  width: 60%;
}

.skeleton-line-long {
  width: 85%;
}

/* 骨架屏闪烁（独立定义，避免污染 ip-spin 关键帧）；
   用 background-position 滑动比 opacity 更柔和，对前庭敏感用户更友好。 */
@keyframes ip-skeleton-shimmer {
  0% {
    background-position: 200% 0;
  }
  100% {
    background-position: -200% 0;
  }
}

@media (prefers-reduced-motion: reduce) {
  .skeleton-line {
    animation: none;
    background-position: 0 0;
  }
}
</style>

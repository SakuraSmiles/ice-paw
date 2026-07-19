<script setup lang="ts">
// W2.4: 聊天状态栏 — 显示 token 用量 / 轮次 / 耗时 / 缓存命中率
//
// 显示位置：聊天区域右上角（浮于 MessageList 之上，半透明毛玻璃背景）
// 可见条件：有 round-state 数据（首次 chat:round-state 事件触发后显示）
// 暗色/亮色主题自适应（使用 --ip-* Design Token）
// W2.6: 重试原因显示（3 秒后自动消失）

import { computed, ref, watch } from "vue";
import { useChatStore } from "../../stores/chat";
import { useConversationsStore } from "../../stores/conversations";

const chatStore = useChatStore();
const conversationsStore = useConversationsStore();

/** 当前会话 ID（来自 conversations store，跨组件共享） */
const currentConversationId = computed<string | null>(
  () => conversationsStore.currentId,
);

/**
 * M1.3 后半：当前会话累计 token（Σ）。
 *
 * 读自 chatStore.conversationTokenUsage map（key=conversation_id），
 * 切换会话 / 应用重启前一直累计。返回 0 表示无累计或无活跃会话。
 */
const sessionTotalTokens = computed<number>(() => {
  const id = currentConversationId.value;
  if (!id) return 0;
  return chatStore.conversationTokenUsage.get(id) ?? 0;
});

/** 当前 round-state 数据（来自 chat:round-state 事件） */
const state = computed(() => chatStore.lastRoundState);

/** 是否可见：有数据且有活跃会话，或有待显示的 finish_reason */
const visible = computed(() => !!state.value || !!finishReasonLabel.value);

/** W4.2: finish_reason 标签（非标准原因才显示） */
const finishReasonLabel = computed(() => {
  const reason = chatStore.lastFinishReason;
  if (!reason || reason === 'stop') return null;
  if (reason === 'budget_exceeded') return 'Token 预算已用尽';
  if (reason === 'length') return '达到 Token 上限';
  if (reason === 'tool_use') return '工具轮数已达上限';
  // M2.1: LLM 连续多轮无进展 → 停滞检测自动终止
  if (reason === 'stuck') return 'LLM 输出停滞，已自动终止';
  return reason;
});

/** 缓存命中率（仅 cached_tokens > 0 时显示） */
const cachePercent = computed(() => {
  const s = state.value;
  if (!s || s.cached_tokens === 0) return null;
  const total = s.tokens_prompt + s.tokens_completion;
  if (total === 0) return null;
  return Math.round((s.cached_tokens / total) * 100);
});

/** 格式化耗时：< 1000ms → "XXXms"，>= 1000ms → "X.Xs" */
const elapsedDisplay = computed(() => {
  const ms = state.value?.elapsed_ms ?? 0;
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
});

// ============================================================================
// W2.6: 重试原因显示
// ============================================================================

/** 最近一次 retry reason（来自 chat:retrying.reason） */
const retryReason = ref<string | null>(null);

/** 重试 reason 定时器 */
let retryTimer: ReturnType<typeof setTimeout> | null = null;

/** 监听 chat:retrying 事件以捕获 reason（由 chat store 的 retrying 状态驱动） */
watch(
  () => chatStore.retrying,
  (retrying, wasRetrying) => {
    // retrying 从 false → true 时，开始显示 reason（如果有）
    if (retrying && !wasRetrying) {
      // reason 从 store 的 lastRoundState 中读取（W2.6 写入）
      // 同时也监听单独的 retry reason 来源
      if (chatStore.lastRoundState?.retry_count && chatStore.retryProgress) {
        // reason 会在 chat:retrying payload 中随 attempt 一起到达
      }
    }
    // retrying 从 true → false 时，隐藏
    if (!retrying) {
      retryReason.value = null;
      if (retryTimer) {
        clearTimeout(retryTimer);
        retryTimer = null;
      }
    }
  },
);

/**
 * W2.6: 显示重试原因（外部调用，传入 reason 字符串）
 * 状态栏右上角显示 reason，3 秒后自动消失。
 */
function showRetryReason(reason: string) {
  retryReason.value = reason;
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = setTimeout(() => {
    retryReason.value = null;
    retryTimer = null;
  }, 3000);
}

// expose for external triggering from ChatPage if needed
defineExpose({ showRetryReason });
</script>

<template>
  <Transition name="status-bar">
    <div
      v-if="visible"
      class="status-bar"
      role="status"
      aria-live="polite"
      aria-label="聊天状态信息"
    >
      <!-- W2.6: 重试原因（右上角，覆盖在状态信息上） -->
      <Transition name="fade">
        <div v-if="retryReason" class="status-retry-reason">
          ⚡ {{ retryReason }}
        </div>
      </Transition>

      <!-- W4.2: finish_reason 非标准提示（如 budget_exceeded） -->
      <Transition name="fade">
        <div v-if="finishReasonLabel" class="status-finish-reason">
          ⚠️ {{ finishReasonLabel }}
        </div>
      </Transition>

      <!-- 主体状态信息 -->
      <div class="status-items">
        <!-- Token 用量 -->
        <span class="status-item status-tokens" title="Token 用量">
          <span class="status-label">Tokens</span>
          <span class="status-val">
            <span class="token-prompt" title="Prompt tokens">P&nbsp;{{ state?.tokens_prompt ?? 0 }}</span>
            <span class="token-sep">/</span>
            <span class="token-completion" title="Completion tokens">C&nbsp;{{ state?.tokens_completion ?? 0 }}</span>
          </span>
        </span>

        <!-- M1.3 后半：会话累计 Token（Σ） -->
        <span class="status-divider" aria-hidden="true">·</span>
        <span class="status-item status-session-total" title="会话累计 Token">
          <span class="status-label">Σ</span>
          <span class="status-val">{{ sessionTotalTokens }}</span>
        </span>

        <!-- M1.5：摘要注入指示器（仅当当前会话发生过摘要压缩时显示） -->
        <template v-if="chatStore.lastSummary && chatStore.lastSummary.conversation_id === currentConversationId">
          <span class="status-divider" aria-hidden="true">·</span>
          <span class="status-item status-summary" title="已压缩历史消息">
            <span class="status-label">📜</span>
            <span class="status-val">已压缩 {{ chatStore.lastSummary.original_count }} 条</span>
          </span>
        </template>

        <span class="status-divider" aria-hidden="true">·</span>

        <!-- 当前轮次 -->
        <span class="status-item" title="当前轮次">
          <span class="status-label">Round</span>
          <span class="status-val">{{ state?.round ?? 0 }}</span>
        </span>

        <span class="status-divider" aria-hidden="true">·</span>

        <!-- 本轮耗时 -->
        <span class="status-item" title="本轮耗时">
          <span class="status-label">Time</span>
          <span class="status-val">{{ elapsedDisplay }}</span>
        </span>

        <!-- 缓存命中率（仅非零时显示） -->
        <template v-if="cachePercent !== null">
          <span class="status-divider" aria-hidden="true">·</span>
          <span class="status-item status-cache" title="缓存命中率">
            <span class="status-cache-icon">⚡</span>
            <span class="status-val">{{ cachePercent }}%</span>
          </span>
        </template>
      </div>
    </div>
  </Transition>
</template>

<style scoped>
/* 整体状态栏 */
.status-bar {
  position: absolute;
  top: 8px;
  right: 12px;
  z-index: 10;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 4px;
  padding: 6px 10px;
  border-radius: var(--ip-radius-md, 8px);
  background: var(--ip-color-bg-tertiary, rgba(0, 0, 0, 0.06));
  backdrop-filter: blur(8px);
  -webkit-backdrop-filter: blur(8px);
  border: 1px solid var(--ip-color-border-default, rgba(0, 0, 0, 0.08));
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.06);
  font-size: 11px;
  line-height: 1;
  color: var(--ip-color-text-tertiary, #6b7280);
  transition: background-color 150ms, opacity 150ms;
  max-width: 320px;
}

.status-bar:hover {
  background: var(--ip-color-bg-tertiary, rgba(0, 0, 0, 0.1));
}

/* 重试原因提示 */
.status-retry-reason {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: var(--ip-radius-sm, 4px);
  background: var(--ip-warning-bg, rgba(251, 191, 36, 0.15));
  color: var(--ip-warning-text, #b45309);
  font-size: 10px;
  font-weight: 500;
}

/* W4.2: finish_reason 非标准提示（如 budget_exceeded） */
.status-finish-reason {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 3px 8px;
  border-radius: var(--ip-radius-sm, 4px);
  background: var(--ip-error-bg, rgba(239, 68, 68, 0.12));
  color: var(--ip-error-text, #dc2626);
  font-size: 10px;
  font-weight: 500;
}

/* 状态项 */
.status-items {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.status-item {
  display: inline-flex;
  align-items: baseline;
  gap: 3px;
}

.status-label {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.4px;
  color: var(--ip-color-text-quaternary, #9ca3af);
  margin-right: 1px;
}

.status-val {
  font-family: var(--ip-font-mono, ui-monospace, "Cascadia Code", "Source Code Pro", Menlo, Consolas, monospace);
  font-size: 11px;
  color: var(--ip-color-text-tertiary, #6b7280);
}

.status-divider {
  color: var(--ip-color-border-strong, #d1d5db);
  font-size: 10px;
}

/* Token 用量特殊样式 */
.status-tokens .status-val {
  display: inline-flex;
  align-items: baseline;
  gap: 2px;
}

.token-prompt {
  color: var(--ip-color-text-tertiary, #6b7280);
}

.token-sep {
  color: var(--ip-color-border-strong, #d1d5db);
  margin: 0 1px;
}

.token-completion {
  color: var(--ip-color-text-tertiary, #6b7280);
}

/* 缓存命中率 */
.status-cache {
  display: inline-flex;
  align-items: baseline;
  gap: 3px;
}

.status-cache-icon {
  font-size: 10px;
  color: var(--ip-primary-500, #3b82f6);
}

.status-cache .status-val {
  color: var(--ip-primary-500, #3b82f6);
  font-weight: 500;
}

/* 会话累计 Token（Σ） */
.status-session-total {
  display: inline-flex;
  align-items: baseline;
  gap: 3px;
}

.status-session-total .status-label {
  color: var(--ip-primary-500, #3b82f6);
  font-weight: 600;
}

.status-session-total .status-val {
  color: var(--ip-primary-500, #3b82f6);
  font-weight: 500;
}

/* M1.5：摘要注入指示器 */
.status-summary {
  display: inline-flex;
  align-items: baseline;
  gap: 3px;
}

.status-summary .status-label {
  font-size: 11px;
  /* emoji 本身已有颜色，不覆盖 */
}

.status-summary .status-val {
  color: var(--ip-color-text-tertiary, #6b7280);
  font-weight: 500;
}

/* ===== 动画 ===== */

/* 状态栏入场/退场 */
.status-bar-enter-active,
.status-bar-leave-active {
  transition: opacity 200ms ease, transform 200ms ease;
}

.status-bar-enter-from,
.status-bar-leave-to {
  opacity: 0;
  transform: translateY(-4px);
}

/* 重试原因淡入淡出 */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 150ms ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

/* 减少动效偏好 */
@media (prefers-reduced-motion: reduce) {
  .status-bar-enter-active,
  .status-bar-leave-active,
  .fade-enter-active,
  .fade-leave-active {
    transition: none;
  }
}
</style>

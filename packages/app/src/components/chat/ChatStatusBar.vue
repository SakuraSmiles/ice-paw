<script setup lang="ts">
// W2.4 / ui-designer: 聊天状态栏 — 悬浮胶囊 + 点击展开详情面板
//
// 显示位置：聊天区域右上角（绝对定位悬浮于 MessageList 之上）
// 可见条件：有 round-state 数据（首次 chat:round-state 事件触发后显示）
//
// 双层结构：
//   1. 默认状态：紧凑胶囊（Tokens P/C · Round · Time）
//   2. 点击胶囊 → 展开详情面板（Σ 累计 · Cache 命中率 · Retry · finish_reason 等）
//
// 关键设计：
//   - 容器 pointer-events:none，胶囊 / 面板 pointer-events:auto
//     → 不阻挡 MessageList 区域的滚动/选择/点击
//   - 用 @vueuse/core 的 onClickOutside 关闭详情面板
//   - 用 useDebounceFn 做 1s 节流，避免每 token 重渲染
//   - 响应式：< 640px 时胶囊只保留 token 数字
//   - 警告态（retry / 非标准 finish_reason）：胶囊变橙色 + 单次脉冲
//     → 自动展开详情面板一次，5s 后自动收回（用户操作可打断）

import { computed, onUnmounted, ref, watch } from "vue";
import { onClickOutside, useDebounceFn, useWindowSize } from "@vueuse/core";
import { Clipboard, Check, TriangleAlert, Zap, ScrollText } from "lucide-vue-next";
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

/** 是否可见：有数据 / 待显示的 finish_reason / 重试中，或当前会话已有 AI 回复过 */
const visible = computed(() => {
  if (state.value || finishReasonLabel.value || retryReason.value) return true;
  // 当前会话有 AI 回复过就保持显示（不再依赖每轮的 round-state 数据）
  return chatStore.messages.some((m) => m.role === "assistant");
});

// ============================================================================
// 派生：紧凑 / 详情文本
// ============================================================================

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

/**
 * 数字千分位压缩：1234 → "1.2k", 12345 → "12k", 1234567 → "1.2m"
 * 紧凑展示用，详情面板里仍展示原始数字。
 */
function compactNumber(n: number): string {
  if (n < 1000) return String(n);
  if (n < 10_000) return `${(n / 1000).toFixed(1)}k`;
  if (n < 1_000_000) return `${Math.round(n / 1000)}k`;
  return `${(n / 1_000_000).toFixed(1)}m`;
}

/** 胶囊用 tokens（P / C 紧凑显示） */
const pillTokensText = computed(() => {
  const s = state.value;
  if (!s) return '0/0';
  return `${compactNumber(s.tokens_prompt)}/${compactNumber(s.tokens_completion)}`;
});

/** 胶囊用 round */
const pillRoundText = computed(() => {
  const r = state.value?.round ?? 0;
  return `R${r}`;
});

/** 胶囊用 elapsed */
const pillTimeText = computed(() => elapsedDisplay.value);

// ============================================================================
// 响应式：< 640px 时胶囊只保留 token 数字
// ============================================================================

const { width } = useWindowSize();
const compact = computed<boolean>(() => width.value < 640);

// ============================================================================
// 详情面板：展开 / 收起 + 警告态自动展开一次
// ============================================================================

/** 详情面板是否展开 */
const panelOpen = ref<boolean>(false);

/** 详情面板 DOM 引用（onClickOutside 用） */
const containerRef = ref<HTMLElement | null>(null);

onClickOutside(containerRef, () => {
  if (panelOpen.value) panelOpen.value = false;
});

/** 点击胶囊 → 切换展开 / 收起，并打断警告态自动收回计时 */
function togglePanel(): void {
  panelOpen.value = !panelOpen.value;
  if (panelOpen.value) {
    // 打断警告态的自动收回
    if (autoCollapseTimer !== null) {
      clearTimeout(autoCollapseTimer);
      autoCollapseTimer = null;
    }
  }
}

// ============================================================================
// 警告态：retry / finish_reason → 橙色 + 单次脉冲 + 自动展开一次
// ============================================================================

const warning = computed<boolean>(
  () => !!retryReason.value || !!finishReasonLabel.value,
);

const pulsing = ref<boolean>(false);

/** 自动展开 / 自动收回计时器 */
let autoCollapseTimer: ReturnType<typeof setTimeout> | null = null;
/** 脉冲动画单次触发计时器 */
let pulseTimer: ReturnType<typeof setTimeout> | null = null;

// ============================================================================
// W2.6: 重试原因显示
// ============================================================================

/** 最近一次 retry reason（来自 chat:retrying 事件） */
const retryReason = ref<string | null>(null);

/** 重试 reason 定时器 */
let retryTimer: ReturnType<typeof setTimeout> | null = null;

/** 监听 chatStore.retrying 变化 → 拉取 reason 并触发警告态 */
watch(
  () => chatStore.retrying,
  (retrying, wasRetrying) => {
    if (retrying && !wasRetrying) {
      // reason 来自 payload（chat:retrying.reason），由 store 写入时附带
      // 此处仅读取最近一次 reason（若有）
    }
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
 * W2.6: 显示重试原因（外部调用，传入 reason 字符串）。
 * 触发警告态：胶囊变橙色 + 单次脉冲 + 自动展开一次，5 秒后自动收回。
 */
function showRetryReason(reason: string): void {
  retryReason.value = reason;

  // 3 秒后自动清除 reason 显示（保留警告态由 finishReason 维持）
  if (retryTimer) clearTimeout(retryTimer);
  retryTimer = setTimeout(() => {
    retryReason.value = null;
    retryTimer = null;
  }, 3000);

  // 触发警告态：单次脉冲 + 自动展开一次 + 5s 后自动收回
  triggerWarningAutoExpand();
}

/**
 * 警告态统一处理：单次脉冲 + 自动展开一次，5s 后自动收回（用户操作可打断）。
 */
function triggerWarningAutoExpand(): void {
  // 单次脉冲
  if (pulseTimer) clearTimeout(pulseTimer);
  pulsing.value = false;
  // 用 nextTick 强制重置 class 以便重新触发动画
  requestAnimationFrame(() => {
    pulsing.value = true;
    pulseTimer = setTimeout(() => {
      pulsing.value = false;
      pulseTimer = null;
    }, 1200);
  });

  // 自动展开一次（若已展开则不重复触发）
  if (!panelOpen.value) {
    panelOpen.value = true;
  }

  // 5 秒后自动收回（用户操作可在 togglePanel 中打断）
  if (autoCollapseTimer) clearTimeout(autoCollapseTimer);
  autoCollapseTimer = setTimeout(() => {
    if (panelOpen.value) panelOpen.value = false;
    autoCollapseTimer = null;
  }, 5000);
}

/** 监听 finish_reason 出现（非 stop）→ 触发警告态 */
watch(finishReasonLabel, (label, oldLabel) => {
  if (label && label !== oldLabel) {
    triggerWarningAutoExpand();
  }
});

// expose for external triggering from ChatPage if needed
defineExpose({ showRetryReason });

// ============================================================================
// 数据更新节流：useDebounceFn 包裹（1s）
//
// 每 token 都会触发 chatStore.lastRoundState 变化，渲染压力较大。
// 用 1s 节流保证 UI 至多每秒更新一次，但数据本身仍然实时变化。
// ============================================================================

/** 节流后的 tokens_prompt（详情面板使用） */
const debouncedTokensPrompt = ref<number>(0);
const debouncedTokensCompletion = ref<number>(0);
const debouncedRound = ref<number>(0);
const debouncedElapsedMs = ref<number>(0);

const pushThrottledSnapshot = useDebounceFn(
  () => {
    const s = state.value;
    debouncedTokensPrompt.value = s?.tokens_prompt ?? 0;
    debouncedTokensCompletion.value = s?.tokens_completion ?? 0;
    debouncedRound.value = s?.round ?? 0;
    debouncedElapsedMs.value = s?.elapsed_ms ?? 0;
  },
  1000,
  // maxWait 等于 wait：保证至少每 1s 更新一次（trailing edge）
);

watch(state, () => pushThrottledSnapshot(), { immediate: true });

// ============================================================================
// 复制明细
// ============================================================================

/** 复制按钮是否刚刚点击过（显示"已复制"反馈） */
const copied = ref<boolean>(false);

/** 复制按钮临时计时器 */
let copyFeedbackTimer: ReturnType<typeof setTimeout> | null = null;

/** 构建明细文本并写入剪贴板 */
async function copyDetails(): Promise<void> {
  const now = new Date();
  const ts = `${now.getFullYear()}-${String(now.getMonth() + 1).padStart(2, "0")}-${String(now.getDate()).padStart(2, "0")} ${String(now.getHours()).padStart(2, "0")}:${String(now.getMinutes()).padStart(2, "0")}`;

  const convTitle = conversationsStore.current?.title ?? "未命名会话";

  const s = state.value;
  const p = s?.tokens_prompt ?? 0;
  const c = s?.tokens_completion ?? 0;
  const totalSession = sessionTotalTokens.value;
  const cache = cachePercent.value;

  const lines: string[] = [
    "IcePaw Token 用量",
    `时间: ${ts}`,
    `会话: ${convTitle}`,
    "",
    "本轮:",
    `  Prompt: ${p} tokens`,
    `  Completion: ${c} tokens`,
    "",
    "会话累计:",
    `  Prompt+Completion: ${totalSession} tokens`,
  ];
  if (cache !== null) {
    lines.push(`  缓存命中: ${cache}%`);
  }

  const text = lines.join("\n");
  try {
    await navigator.clipboard.writeText(text);
    copied.value = true;
    if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer);
    copyFeedbackTimer = setTimeout(() => {
      copied.value = false;
      copyFeedbackTimer = null;
    }, 2000);
  } catch {
    // clipboard API 不可用（非 HTTPS / 权限拒绝）→ 静默忽略
  }
}

// 卸载时清理定时器
onUnmounted(() => {
  if (autoCollapseTimer) clearTimeout(autoCollapseTimer);
  if (pulseTimer) clearTimeout(pulseTimer);
  if (retryTimer) clearTimeout(retryTimer);
  if (copyFeedbackTimer) clearTimeout(copyFeedbackTimer);
});
</script>

<template>
  <Transition name="status-fade">
    <div
      v-if="visible"
      ref="containerRef"
      class="status-container"
      :class="{ 'status-container--warning': warning }"
      role="status"
      aria-live="polite"
      aria-label="聊天状态信息"
    >
      <!-- 紧凑胶囊（默认显示） -->
      <button
        type="button"
        class="status-pill"
        :class="{ 'status-pill--pulse': pulsing }"
        :aria-expanded="panelOpen"
        :aria-label="`聊天状态：${pillTokensText} · ${pillRoundText} · ${pillTimeText}。点击查看详情`"
        @click="togglePanel"
      >
        <!-- 紧凑（< 640px）：只保留 token 数字 -->
        <template v-if="compact">
          <span class="pill-tokens">{{ pillTokensText }}</span>
        </template>
        <!-- 完整：Tokens P/C · Round · Time -->
        <template v-else>
          <span class="pill-tokens">
            <span class="pill-tokens-p">P&nbsp;{{ state?.tokens_prompt ?? 0 }}</span>
            <span class="pill-tokens-sep">/</span>
            <span class="pill-tokens-c">C&nbsp;{{ state?.tokens_completion ?? 0 }}</span>
          </span>
          <span class="pill-dot" aria-hidden="true">·</span>
          <span class="pill-round">{{ pillRoundText }}</span>
          <span class="pill-dot" aria-hidden="true">·</span>
          <span class="pill-time">{{ pillTimeText }}</span>
        </template>
        <!-- 警告态小角标（finish_reason / retry 时显示） -->
        <span v-if="warning" class="pill-warn-dot" aria-hidden="true" />
      </button>

      <!-- 详情面板（点击展开） -->
      <Transition name="status-panel">
        <div v-if="panelOpen" class="status-panel" role="dialog" aria-label="聊天状态详情">
          <!-- 当前轮 -->
          <section class="panel-section">
            <header class="panel-section-title">本轮</header>
            <div class="panel-grid">
              <div class="panel-row">
                <span class="panel-label">Tokens</span>
                <span class="panel-val">
                  <span class="panel-token-p">P {{ debouncedTokensPrompt }}</span>
                  <span class="panel-token-sep">/</span>
                  <span class="panel-token-c">C {{ debouncedTokensCompletion }}</span>
                </span>
              </div>
              <div class="panel-row">
                <span class="panel-label">Round</span>
                <span class="panel-val">{{ debouncedRound }}</span>
              </div>
              <div class="panel-row">
                <span class="panel-label">Time</span>
                <span class="panel-val">{{ elapsedDisplay }}</span>
              </div>
              <div v-if="finishReasonLabel" class="panel-row panel-row-warn">
                <span class="panel-label">Finish</span>
                <span class="panel-val"><TriangleAlert :size="12" class="inline" aria-hidden="true" /> {{ finishReasonLabel }}</span>
              </div>
            </div>
          </section>

          <!-- 会话累计 -->
          <section class="panel-section">
            <header class="panel-section-title">会话累计</header>
            <div class="panel-grid">
              <div class="panel-row">
                <span class="panel-label">Σ Tokens</span>
                <span class="panel-val panel-val-accent">{{ sessionTotalTokens }}</span>
              </div>
              <div v-if="cachePercent !== null" class="panel-row">
                <span class="panel-label">Cache</span>
                <span class="panel-val panel-val-accent"><Zap :size="12" class="inline" aria-hidden="true" /> {{ cachePercent }}%</span>
              </div>
              <div v-if="chatStore.lastSummary && chatStore.lastSummary.conversation_id === currentConversationId" class="panel-row">
                <span class="panel-label"><ScrollText :size="12" class="inline" aria-hidden="true" /></span>
                <span class="panel-val">已压缩 {{ chatStore.lastSummary.original_count }} 条</span>
              </div>
            </div>
          </section>

          <!-- 复制明细按钮 -->
          <button
            type="button"
            class="panel-copy-btn"
            :class="{ 'panel-copy-btn--done': copied }"
            @click="copyDetails"
          >
            <component :is="copied ? Check : Clipboard" :size="14" aria-hidden="true" />
            <span>{{ copied ? '已复制' : '复制明细' }}</span>
          </button>

          <!-- 状态（retry） -->
          <section v-if="retryReason" class="panel-section">
            <header class="panel-section-title">状态</header>
            <div class="panel-grid">
              <div class="panel-row panel-row-warn">
                <span class="panel-label">Retry</span>
                <span class="panel-val"><Zap :size="12" class="inline" aria-hidden="true" /> {{ retryReason }} · {{ chatStore.retryProgress }}</span>
              </div>
            </div>
          </section>
        </div>
      </Transition>
    </div>
  </Transition>
</template>

<style scoped>
/* ============================================================================
 * 容器：pointer-events: none，不阻挡消息区滚动 / 选择 / 点击
 * 胶囊和面板：pointer-events: auto，自己处理交互
 * ============================================================================ */
.status-container {
  position: absolute;
  top: 10px;
  right: 12px;
  z-index: 20;
  pointer-events: none;
  display: flex;
  flex-direction: column;
  align-items: flex-end;
  gap: 6px;
}

/* ============================================================================
 * 紧凑胶囊
 * ============================================================================ */
.status-pill {
  pointer-events: auto;
  display: inline-flex;
  align-items: center;
  gap: 6px;
  height: 28px;
  padding: 0 12px;
  border-radius: 14px;
  background: rgba(14, 22, 38, 0.78);
  backdrop-filter: blur(12px);
  -webkit-backdrop-filter: blur(12px);
  border: 1px solid rgba(125, 211, 252, 0.18);
  box-shadow: 0 2px 6px rgba(0, 0, 0, 0.18);
  font-family: var(--ip-font-mono, ui-monospace, "Cascadia Code", "Source Code Pro", Menlo, Consolas, monospace);
  font-size: 12px;
  line-height: 1;
  color: rgba(226, 232, 240, 0.86);
  cursor: pointer;
  user-select: none;
  transition: background-color 150ms ease, transform 150ms ease, border-color 150ms ease;
  /* 默认 button 样式重置 */
  appearance: none;
  outline: none;
}

.status-pill:hover {
  background: rgba(14, 22, 38, 0.92);
  transform: translateY(-1px);
  border-color: rgba(125, 211, 252, 0.32);
}

.status-pill:focus-visible {
  border-color: rgba(125, 211, 252, 0.55);
  box-shadow: 0 0 0 2px rgba(125, 211, 252, 0.25);
}

/* 胶囊内的数字：使用 #7DD3FC 高亮 */
.pill-tokens-p,
.pill-tokens-c,
.pill-round,
.pill-time {
  color: #7DD3FC;
  font-weight: 500;
}

.pill-tokens-sep,
.pill-dot {
  color: rgba(148, 163, 184, 0.5);
  margin: 0 1px;
}

.pill-warn-dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  margin-left: 4px;
  border-radius: 50%;
  background: #f59e0b;
  box-shadow: 0 0 6px rgba(245, 158, 11, 0.7);
}

/* 警告态：胶囊变橙色 */
.status-container--warning .status-pill {
  background: rgba(245, 158, 11, 0.15);
  border-color: rgba(245, 158, 11, 0.45);
}

.status-container--warning .pill-warn-dot {
  background: #fbbf24;
}

/* 单次脉冲动画 */
.status-pill--pulse {
  animation: pill-pulse 1.2s ease-out 1;
}

@keyframes pill-pulse {
  0% {
    box-shadow: 0 0 0 0 rgba(245, 158, 11, 0.55);
  }
  60% {
    box-shadow: 0 0 0 10px rgba(245, 158, 11, 0);
  }
  100% {
    box-shadow: 0 0 0 0 rgba(245, 158, 11, 0);
  }
}

/* ============================================================================
 * 详情面板
 * ============================================================================ */
.status-panel {
  pointer-events: auto;
  width: 320px;
  max-width: calc(100vw - 24px);
  padding: 12px;
  border-radius: 12px;
  background: rgba(20, 28, 44, 0.85);
  backdrop-filter: blur(20px);
  -webkit-backdrop-filter: blur(20px);
  border: 1px solid rgba(125, 211, 252, 0.18);
  box-shadow: 0 8px 24px rgba(0, 0, 0, 0.32);
  font-family: var(--ip-font-mono, ui-monospace, "Cascadia Code", "Source Code Pro", Menlo, Consolas, monospace);
  font-size: 12px;
  line-height: 1.5;
  color: rgba(226, 232, 240, 0.92);
  display: flex;
  flex-direction: column;
  gap: 10px;
  /* 起点：scale(0.95) + fade（由 .status-panel-enter-from 控制） */
  transform-origin: top right;
}

.panel-section {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.panel-section-title {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.6px;
  color: rgba(148, 163, 184, 0.7);
  font-weight: 600;
}

.panel-grid {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.panel-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.panel-row-warn .panel-val {
  color: #fbbf24;
}

.panel-label {
  font-size: 11px;
  color: rgba(148, 163, 184, 0.8);
}

.panel-val {
  font-size: 12px;
  color: rgba(226, 232, 240, 0.92);
  text-align: right;
}

.panel-val-accent {
  color: #7DD3FC;
  font-weight: 500;
}

.panel-token-p,
.panel-token-c {
  color: #7DD3FC;
}

.panel-token-sep {
  color: rgba(148, 163, 184, 0.5);
  margin: 0 2px;
}

/* ============================================================================
 * 复制明细按钮
 * ============================================================================ */
.panel-copy-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  width: 100%;
  height: 30px;
  border-radius: 8px;
  border: 1px solid rgba(125, 211, 252, 0.18);
  background: rgba(125, 211, 252, 0.06);
  color: rgba(226, 232, 240, 0.86);
  font-family: inherit;
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  appearance: none;
  outline: none;
  transition:
    background-color 150ms ease,
    border-color 150ms ease,
    color 150ms ease;
}

.panel-copy-btn:hover {
  background: rgba(125, 211, 252, 0.14);
  border-color: rgba(125, 211, 252, 0.32);
}

.panel-copy-btn:focus-visible {
  box-shadow: 0 0 0 2px rgba(125, 211, 252, 0.25);
}

.panel-copy-btn--done {
  border-color: rgba(74, 222, 128, 0.4);
  background: rgba(74, 222, 128, 0.1);
  color: #4ade80;
}

/* ============================================================================
 * 动画
 * ============================================================================ */

/* 整体淡入淡出 */
.status-fade-enter-active,
.status-fade-leave-active {
  transition: opacity 200ms ease;
}

.status-fade-enter-from,
.status-fade-leave-to {
  opacity: 0;
}

/* 详情面板入场：scale(0.95) → scale(1) + fade */
.status-panel-enter-active {
  transition: opacity 180ms ease, transform 180ms cubic-bezier(0.16, 1, 0.3, 1);
}

.status-panel-leave-active {
  transition: opacity 120ms ease, transform 120ms ease;
}

.status-panel-enter-from {
  opacity: 0;
  transform: scale(0.95);
}

.status-panel-leave-to {
  opacity: 0;
  transform: scale(0.97);
}

/* 减少动效偏好 */
@media (prefers-reduced-motion: reduce) {
  .status-fade-enter-active,
  .status-fade-leave-active,
  .status-panel-enter-active,
  .status-panel-leave-active,
  .status-pill--pulse {
    transition: none;
    animation: none;
  }
}

/* ============================================================================
 * backdrop-filter 降级（不支持时回退到更不透明的纯色）
 * ============================================================================ */
@supports not (backdrop-filter: blur(12px)) {
  .status-pill {
    background: rgba(14, 22, 38, 0.95);
  }
  .status-panel {
    background: rgba(20, 28, 44, 0.98);
  }
}
</style>
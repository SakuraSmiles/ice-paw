<script setup lang="ts">
// 聊天主页面（Phase 1 P0 模块 3 前端）
//
// 布局：
//   - 顶部：ChatHeader（会话标题 + Agent 信息 + 停止按钮）
//   - 中部：MessageList（flex: 1，可滚动）
//   - 底部：ChatInput（多行输入 + 工具栏）
//
// 行为：
//   - onMounted：注册 4 个 chat:* 事件监听（chatStore.setupListeners）
//   - 监听 conversationsStore.currentId 变化：调 chatStore.loadMessages 拉历史
//   - 无当前会话：显示「选择或创建一个会话开始聊天」全屏空状态
//   - 有当前会话但无消息：显示 EmptyChatHint
//   - 错误：通过 Toast 提示（chatStore.error）
//
// 重试功能：P1 占位 — 目前仅 Toast 提示「重试功能开发中」

import { computed, onMounted, onUnmounted, watch } from "vue";
import { useConversationsStore } from "../stores/conversations";
import { useChatStore } from "../stores/chat";
import { useToast } from "../composables/useToast";
import ChatHeader from "../components/chat/ChatHeader.vue";
import MessageList from "../components/chat/MessageList.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import EmptyChatHint from "../components/chat/EmptyChatHint.vue";

const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const toast = useToast();

// ============================================================================
// 派生状态
// ============================================================================

/** 当前是否有选中会话 */
const hasConversation = computed<boolean>(() => !!conversationsStore.currentId);

/** 当前会话 ID（用于触发 watch） */
const currentConvId = computed<string | null>(() => conversationsStore.currentId);

/** 流式中助手消息 ID（取 messages 末尾且 role=assistant 且无 error 的项） */
const streamingMessageId = computed<string | null>(() => {
  if (!chatStore.isStreaming) return null;
  const list = chatStore.messages;
  for (let i = list.length - 1; i >= 0; i--) {
    const m = list[i];
    if (m && m.role === "assistant" && !m.error) return m.id;
  }
  return null;
});

// ============================================================================
// 生命周期
// ============================================================================

onMounted(async () => {
  // 注册 4 个 chat:* 事件监听
  await chatStore.setupListeners();

  // 若已有当前会话，立刻拉历史（处理冷启动）
  if (currentConvId.value) {
    try {
      await chatStore.loadMessages(currentConvId.value);
    } catch {
      toast.error("加载历史消息失败");
    }
  }
});

onUnmounted(() => {
  // 注销监听，避免内存泄漏
  chatStore.teardownListeners();
});

// ============================================================================
// 监听：currentConvId 变化时拉历史
// ============================================================================

watch(currentConvId, async (newId, oldId) => {
  if (newId === oldId) return;
  if (!newId) {
    // 切换到无会话：清空消息
    chatStore.messages.length = 0;
    chatStore.clearError();
    return;
  }
  try {
    await chatStore.loadMessages(newId);
  } catch {
    toast.error("加载历史消息失败");
  }
});

// ============================================================================
// 监听：chatStore.error 变化触发 Toast
// ============================================================================

watch(
  () => chatStore.error,
  (msg) => {
    if (msg) {
      toast.error(msg);
      chatStore.clearError();
    }
  },
);

// ============================================================================
// 事件处理
// ============================================================================

/** 用户点击发送 */
async function onSend(content: string): Promise<void> {
  try {
    await chatStore.sendMessage(content);
  } catch {
    // sendMessage 内部已写入 error，watch 会兜底弹 Toast
  }
}

/** 用户点击停止 */
async function onStop(): Promise<void> {
  await chatStore.stopGeneration();
}

/** 用户点击重试（P1 占位） */
function onRetry(_msg: import("../types").Message): void {
  toast.info("重试功能开发中");
}
</script>

<template>
  <div class="chat-page">
    <!-- 无当前会话：全屏空状态 -->
    <div v-if="!hasConversation" class="no-conv">
      <div class="no-conv-card">
        <h2 class="no-conv-title">选择或创建一个会话开始聊天</h2>
        <p class="no-conv-desc">在左侧侧栏选择一个已有会话，或点击「+ 新建会话」开始新的对话。</p>
      </div>
    </div>

    <!-- 有当前会话：完整聊天界面 -->
    <template v-else>
      <ChatHeader @stop="onStop" />
      <MessageList
        :messages="chatStore.currentMessages"
        :streaming-id="streamingMessageId"
        @retry="onRetry"
      />
      <div v-if="chatStore.isEmpty" class="empty-overlay-wrap">
        <EmptyChatHint />
      </div>
      <ChatInput
        :disabled="false"
        :streaming="chatStore.isStreaming"
        @send="onSend"
        @stop="onStop"
      />
    </template>
  </div>
</template>

<style scoped>
.chat-page {
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  background: var(--ip-color-bg-secondary);
  overflow: hidden;
}

.no-conv {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 48px var(--ip-spacing-6);
}

.no-conv-card {
  max-width: 420px;
  text-align: center;
  padding: var(--ip-spacing-8) var(--ip-spacing-6);
  border: 1px dashed var(--ip-color-border-default);
  border-radius: var(--ip-radius-xl);
  background: var(--ip-color-bg-elevated);
}

.no-conv-title {
  margin: 0 0 var(--ip-spacing-2);
  font-size: var(--ip-text-body-lg-size);
  font-weight: var(--ip-font-weight-semibold);
  color: var(--ip-color-text-primary);
}

.no-conv-desc {
  margin: 0;
  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose);
  color: var(--ip-color-text-tertiary);
}

.empty-overlay-wrap {
  position: absolute;
  inset: 56px 0 0; /* 减去 header 高度 56px */
  display: flex;
  pointer-events: none;
}

.empty-overlay-wrap > * {
  flex: 1;
  pointer-events: auto;
}
</style>

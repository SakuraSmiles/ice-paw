<script setup lang="ts">
// 聊天主页面（Phase 1 P0 模块 3 前端）
//
// 三态渲染：
//   1. hasAgents === false            → InlineAgentCreate（首页内联创建）
//   2. hasAgents && !hasConversation  → WelcomeScreen（居中头像 + 提示词 + WelcomeInput）
//   3. hasConversation                → 完整聊天界面（ChatHeader + MessageList + ChatInput）
//
// 行为：
//   - onMounted：注册 4 个 chat:* 事件监听（chatStore.setupListeners）
//   - 监听 conversationsStore.currentId 变化：调 chatStore.loadMessages 拉历史
//   - 首条消息即创建会话：由 WelcomeInput 自动创建后再 sendMessage
//   - 错误：通过 Toast 提示（chatStore.error）
//
// 重试功能：P1 占位 — 目前仅 Toast 提示「重试功能开发中」

import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useAgentsStore } from "../stores/agents";
import { useConversationsStore } from "../stores/conversations";
import { useChatStore } from "../stores/chat";
import { useToast } from "../composables/useToast";
import { useToolAuth } from "../composables/useToolAuth";
import ChatHeader from "../components/chat/ChatHeader.vue";
import MessageList from "../components/chat/MessageList.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import WelcomeInput from "../components/chat/WelcomeInput.vue";
import InlineAgentCreate from "../components/agent/InlineAgentCreate.vue";
import ChatStatusBar from "../components/chat/ChatStatusBar.vue";
import ToolAuthDialog from "../components/chat/ToolAuthDialog.vue";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const toast = useToast();
const toolAuth = useToolAuth();

// ============================================================================
// 派生状态
// ============================================================================

/** 当前是否有 Agent（用于三态分支的第一道门） */
const hasAgents = computed<boolean>(() => agentsStore.hasAgents);

/** 当前是否有选中会话 */
const hasConversation = computed<boolean>(() => !!conversationsStore.currentId);

/** 当前会话 ID（用于触发 watch） */
const currentConvId = computed<string | null>(() => conversationsStore.currentId);

/** 当前 Agent 名称（WelcomeScreen 用） */
const currentAgentName = computed<string>(() => agentsStore.current?.name ?? "");

/** 当前 Agent 模型名（WelcomeScreen 用） */
const currentAgentModel = computed<string>(() => agentsStore.current?.model ?? "");

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

/**
 * MessageList 组件引用（用于调用其暴露的 forceBottom()）。
 *
 * 切换会话后需要让列表"立即定位到底部"，由于 list 内部默认用
 * pinnedToBottom 维持底部，新会话还没"被观察到用户就在底部"，所以
 * 单纯依赖 watch 会出现「先看到顶部再往下滚」的闪烁感。
 * 通过这个 ref 显式调用 forceBottom() 跳过 pinnedToBottom 判定。
 */
const messageListRef = ref<InstanceType<typeof MessageList> | null>(null);

/**
 * 强制 MessageList 滚动到底部（包装 forceBottom）。
 * 内部还会做二次 nextTick 兜底 v-for patch 时序。
 */
async function scrollMessageListToBottom(): Promise<void> {
  // 等 Vue 把 messages 数组的变化提交到 DOM
  await nextTick();
  messageListRef.value?.forceBottom();
}

// ============================================================================
// 生命周期
// ============================================================================

onMounted(async () => {
  // 注册 4 个 chat:* 事件监听
  await chatStore.setupListeners();
  // A2-3: 注册工具授权请求监听
  await toolAuth.setupAuthListener();

  // 若已有当前会话，立刻拉历史（处理冷启动）并立即滚到底部
  if (currentConvId.value) {
    try {
      await chatStore.loadMessages(currentConvId.value);
      await scrollMessageListToBottom();
    } catch {
      toast.error("加载历史消息失败");
    }
  }
});

onUnmounted(() => {
  // 注销监听，避免内存泄漏
  chatStore.teardownListeners();
  // A2-3: 注销授权监听 + 清空未响应的请求
  toolAuth.teardownAuthListener();
});

// ============================================================================
// 监听：currentConvId 变化时拉历史
// ============================================================================

watch(currentConvId, async (newId, oldId) => {
  if (newId === oldId) return;
  if (!newId) {
    // 切换到无会话：清空消息
    chatStore.messages.splice(0);
    chatStore.clearError();
    return;
  }
  try {
    await chatStore.loadMessages(newId);
    // P2: 加载完成后立即强制滚到底部（不等 MessageList 的 watch 反应）
    // 用 nextTick + forceBottom 走 messageListRef，避免全局 querySelector
    await scrollMessageListToBottom();
  } catch {
    toast.error("加载历史消息失败");
  }
});

/**
 * 防御性 watch：直接监听 conversationsStore.currentId，
 * 作为 currentConvId 计算之外的兜底。
 *
 * 场景：loadFor() 与 create() 之间的时序竞争 —— store 内部
 * create() 完成后 currentId 已变化，但 ChatPage 因路由切换/重渲
 * 错过了 currentConvId 的触发时机。此 watch 保证：一旦 currentId
 * 变为有效会话且消息列表为空，便主动加载历史消息。
 *
 * 注意：此处只补一次「从无到有」的加载，不重复滚到底部，避免与主 watch
 * 双重 forceBottom 引起闪烁。
 */
watch(
  () => conversationsStore.currentId,
  async (id) => {
    if (!id || chatStore.isStreaming) return;
    if (chatStore.messages.length > 0) return;
    try {
      await chatStore.loadMessages(id);
      await scrollMessageListToBottom();
    } catch {
      // 主 watch 已 Toast 提示，避免重复弹窗
    }
  },
);

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

/** 正常聊天界面：用户点击发送 */
async function onSend(
  content: string,
  contentBlocks?: import("../types").ContentBlock[],
): Promise<void> {
  try {
    await chatStore.sendMessage(content, contentBlocks);
  } catch {
    // sendMessage 内部已写入 error，watch 会兜底弹 Toast
  }
}

/** WelcomeScreen 触发：会话已在 WelcomeInput 内创建完成，此处只需发送 */
async function onWelcomeSend(
  content: string,
  contentBlocks?: import("../types").ContentBlock[],
): Promise<void> {
  // 此时 conversationsStore.currentId 应已被设置
  // 但 loadMessages 还未触发（conversations.watchAgentChange 会自动触发 loadFor，
  // 但 store 手动 setCurrent 不触发 watch）；此处显式加载一次。
  const convId = conversationsStore.currentId;
  if (!convId) {
    toast.error("会话未就绪，请稍后再试");
    return;
  }
  // 确保消息列表已加载（首次进入会话）
  if (chatStore.messages.length === 0 && !chatStore.isStreaming) {
    try {
      await chatStore.loadMessages(convId);
      await scrollMessageListToBottom();
    } catch {
      // 加载失败仍尝试发送（极端情况下允许发送）
    }
  }
  await onSend(content, contentBlocks);
}

/** 用户点击停止 */
async function onStop(): Promise<void> {
  await chatStore.stopGeneration();
}

/** 用户点击重试（P1 占位） */
function onRetry(_msg: import("../types").Message): void {
  toast.info("重试功能开发中");
}

/** P2: 用户滚到顶部附近 → 触发向上翻页 */
async function onLoadOlder(): Promise<void> {
  await chatStore.loadOlderMessages();
}
</script>

<template>
  <div class="chat-page">
    <!-- 三态一：无 Agent → 首页内联创建 -->
    <InlineAgentCreate v-if="!hasAgents && !agentsStore.loading" />

    <!-- 三态二：有 Agent 无会话 → WelcomeScreen（居中输入框 + 提示词） -->
    <WelcomeInput
      v-else-if="hasAgents && !hasConversation"
      :agent-name="currentAgentName"
      :model-name="currentAgentModel"
      @send="onWelcomeSend"
      @stop="onStop"
    />

    <!-- 三态三：有会话 → 完整聊天界面 -->
    <template v-else>
      <ChatHeader @stop="onStop" />
      <div class="message-list-wrapper">
        <MessageList
        ref="messageListRef"
        :messages="chatStore.currentMessages"
        :streaming-id="streamingMessageId"
        :is-retrying="chatStore.retrying"
        :retry-progress="chatStore.retryProgress"
        :active-tool-calls="chatStore.activeToolCalls"
        :thinking-content="chatStore.thinkingContent"
        :has-more-older="chatStore.hasMoreOlder"
        :loading-older="chatStore.loadingOlder"
        :loading="chatStore.loading"
        @retry="onRetry"
        @load-older="onLoadOlder"
      />
      </div>
      <!-- W2.4: 状态栏（浮于 MessageList 区域右上角） -->
      <ChatStatusBar />
      <ChatInput
        :disabled="false"
        :streaming="chatStore.isStreaming"
        @send="onSend"
        @stop="onStop"
      />
    </template>

    <!-- A2-3: 工具授权确认弹窗（全局唯一实例） -->
    <ToolAuthDialog />
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

/* W2.4: 消息列表包装器（用于定位状态栏） */
.message-list-wrapper {
  position: relative;
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
</style>
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

import { computed, onMounted, onUnmounted, watch } from "vue";
import { useAgentsStore } from "../stores/agents";
import { useConversationsStore } from "../stores/conversations";
import { useChatStore } from "../stores/chat";
import { useToast } from "../composables/useToast";
import ChatHeader from "../components/chat/ChatHeader.vue";
import MessageList from "../components/chat/MessageList.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import WelcomeInput from "../components/chat/WelcomeInput.vue";
import InlineAgentCreate from "../components/agent/InlineAgentCreate.vue";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const toast = useToast();

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

/** 正常聊天界面：用户点击发送 */
async function onSend(content: string): Promise<void> {
  try {
    await chatStore.sendMessage(content);
  } catch {
    // sendMessage 内部已写入 error，watch 会兜底弹 Toast
  }
}

/** WelcomeScreen 触发：会话已在 WelcomeInput 内创建完成，此处只需发送 */
async function onWelcomeSend(content: string): Promise<void> {
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
    } catch {
      // 加载失败仍尝试发送（极端情况下允许发送）
    }
  }
  await onSend(content);
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
      <MessageList
        :messages="chatStore.currentMessages"
        :streaming-id="streamingMessageId"
        @retry="onRetry"
      />
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
</style>
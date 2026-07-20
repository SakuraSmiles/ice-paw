<script setup lang="ts">
// 聊天主页面（Phase 2 版）
//
// 三态渲染：
//   1. hasAgents === false            → InlineAgentCreate（首页内联创建）
//   2. hasAgents && !hasConversation  → ProjectWelcome（项目级欢迎页）
//   3. hasConversation                → 完整聊天界面（ChatHeader + MessageList + ChatInput）
//
// 行为：
//   - onMounted：注册 4 个 chat:* 事件监听（chatStore.setupListeners）
//   - 监听 conversationsStore.currentId 变化：调 chatStore.loadMessages 拉历史
//   - ProjectWelcome 点击「新建会话」→ onProjectCreate 创建会话
//   - 错误：通过 Toast 提示（chatStore.error）

import { computed, nextTick, onMounted, onUnmounted, ref, watch } from "vue";
import { useAgentsStore } from "../stores/agents";
import { useConversationsStore } from "../stores/conversations";
import { useChatStore } from "../stores/chat";
import { useProjectsStore, DEFAULT_PROJECT_ID } from "../stores/projects";
import { useToast } from "../composables/useToast";
import { useToolAuth } from "../composables/useToolAuth";
import { ACCEPT_MIMES } from "../composables/useImageFiles";
import ChatHeader from "../components/chat/ChatHeader.vue";
import MessageList from "../components/chat/MessageList.vue";
import ChatInput from "../components/chat/ChatInput.vue";
import ProjectWelcome from "../components/chat/ProjectWelcome.vue";
import InlineAgentCreate from "../components/agent/InlineAgentCreate.vue";
import ChatStatusBar from "../components/chat/ChatStatusBar.vue";
import ToolAuthDialog from "../components/chat/ToolAuthDialog.vue";
import TemplateCards from "../components/chat/TemplateCards.vue";
import DragOverlay from "../components/chat/DragOverlay.vue";

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const projectsStore = useProjectsStore();
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
 * ChatInput 组件引用（用于调用 setDraft 填入模板内容）。
 */
const chatInputRef = ref<InstanceType<typeof ChatInput> | null>(null);

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

/** ProjectWelcome: 点击「新建会话」 */
async function onProjectCreate(): Promise<void> {
  const currentProject = projectsStore.current;
  const isDefault = projectsStore.currentId === DEFAULT_PROJECT_ID;
  const projectAgents = currentProject?.agents ?? [];

  let agentId: string | null = null;

  // 优先使用项目 Agent 成员中的 lead
  if (projectAgents.length > 0) {
    const lead = projectAgents.find((a) => a.role === "lead");
    agentId = lead?.agent_id ?? projectAgents[0]!.agent_id;
  }

  // 默认项目回退到 currentId
  if (!agentId && isDefault) {
    agentId = agentsStore.currentId;
  }

  // 回退到第一个 Agent
  if (!agentId && agentsStore.hasAgents) {
    agentId = agentsStore.agents[0]!.id;
  }

  if (!agentId) {
    toast.warning("请先创建一个 Agent");
    return;
  }

  try {
    await conversationsStore.create(agentId, projectsStore.currentId);
  } catch {
    toast.error("新建会话失败");
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

/** P2: 用户滚到顶部附近 → 触发向上翻页 */
async function onLoadOlder(): Promise<void> {
  await chatStore.loadOlderMessages();
}

/** 模板卡片点击 → 填入 ChatInput 的 draft */
function onTemplateSelect(content: string): void {
  chatInputRef.value?.setDraft(content);
}

/** 是否显示模板卡片空状态（有会话但无消息且非加载中） */
const showTemplateCards = computed<boolean>(
  () =>
    hasConversation.value &&
    chatStore.messages.length === 0 &&
    !chatStore.loading &&
    !chatStore.isStreaming,
);

// ============================================================================
// Task 3a: 拖拽上传
// ============================================================================

/** 是否显示拖拽 overlay */
const isDragOver = ref<boolean>(false);

/** 拖拽进入的计数（解决子元素 dragenter/dragleave 冒泡问题） */
const dragCounter = ref<number>(0);

/** 拖拽区域 ref（聊天页面根元素） */
const chatDropZoneRef = ref<HTMLElement | null>(null);

/** 拖拽后文件处理：直接写入 ChatInput 的 pendingImages */
function handleDroppedFiles(files: File[]): void {
  // 通过 ChatInput 暴露的方法处理拖拽文件
  chatInputRef.value?.addFiles?.(files);
}

/** 检查拖拽内容是否包含文件 */
function hasFiles(e: DragEvent): boolean {
  if (!e.dataTransfer) return false;
  const types = e.dataTransfer.types;
  return types.includes("Files");
}

/** dragenter */
function onDragEnter(e: DragEvent): void {
  if (!hasFiles(e)) return;
  e.preventDefault();
  dragCounter.value++;
  isDragOver.value = true;
}

/** dragover — 必须 preventDefault 才能触发 drop */
function onDragOver(e: DragEvent): void {
  if (!hasFiles(e)) return;
  e.preventDefault();
  if (e.dataTransfer) {
    e.dataTransfer.dropEffect = "copy";
  }
}

/** dragleave — 使用 dragCounter 解决子元素冒泡问题 */
function onDragLeave(e: DragEvent): void {
  if (!hasFiles(e)) return;
  e.preventDefault();
  dragCounter.value--;
  if (dragCounter.value <= 0) {
    dragCounter.value = 0;
    isDragOver.value = false;
  }
}

/** drop — 提取文件并处理 */
function onDrop(e: DragEvent): void {
  if (!hasFiles(e)) return;
  e.preventDefault();
  dragCounter.value = 0;
  isDragOver.value = false;

  const files = e.dataTransfer?.files;
  if (!files || files.length === 0) return;

  // 过滤出图片文件
  const imageFiles = Array.from(files).filter((f) =>
    ACCEPT_MIMES.includes(f.type as (typeof ACCEPT_MIMES)[number]),
  );

  if (imageFiles.length === 0) {
    toast.warning("仅支持拖拽图片文件（png/jpeg/gif/webp）");
    return;
  }

  void handleDroppedFiles(imageFiles);
}
</script>

<template>
  <div
    ref="chatDropZoneRef"
    class="chat-page"
    @dragenter="onDragEnter"
    @dragover="onDragOver"
    @dragleave="onDragLeave"
    @drop="onDrop"
  >
    <!-- 三态一：无 Agent → 首页内联创建 -->
    <InlineAgentCreate v-if="!hasAgents && !agentsStore.loading" />

    <!-- 三态二：有 Agent 无会话 → ProjectWelcome（项目级欢迎页） -->
    <ProjectWelcome
      v-else-if="hasAgents && !hasConversation"
      @create="onProjectCreate"
    />

    <!-- 三态三：有会话 → 完整聊天界面 -->
    <template v-else>
      <ChatHeader @stop="onStop" />
      <div class="message-list-wrapper">
        <!-- 空状态：有会话但无消息时显示模板卡片 -->
        <TemplateCards
          v-if="showTemplateCards"
          @select="onTemplateSelect"
        />
        <MessageList
          v-else
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
        <!-- W2.4: 状态栏（浮于 MessageList 区域右上角） -->
        <ChatStatusBar />
      </div>
      <ChatInput
        ref="chatInputRef"
        :disabled="false"
        :streaming="chatStore.isStreaming"
        @send="onSend"
        @stop="onStop"
      />
    </template>

    <!-- A2-3: 工具授权确认弹窗（全局唯一实例） -->
    <ToolAuthDialog />

    <!-- Task 3a: 拖拽上传 overlay -->
    <DragOverlay v-if="isDragOver" />
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

/* 消息列表包装器（用于定位状态栏）。
   注意：状态栏已改为绝对定位悬浮 + pointer-events:none，
   不再需要为状态栏预留 padding-top。 */
.message-list-wrapper {
  position: relative;
  flex: 1;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}
</style>
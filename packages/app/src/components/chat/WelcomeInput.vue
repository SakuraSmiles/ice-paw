<script setup lang="ts">
// 首页居中输入框组件（WelcomeScreen 专用）
//
// 职责：
//   - 居中显示一个大尺寸的多行输入框
//   - 顶部显示 Agent 头像（字母缩写色块 / Lucide 图标）+ 个性化问候语
//   - 下方展示可点击的提示词芯片（优先 meta.promptChips，降级 model 匹配）
//   - Enter 发送；Shift+Enter 换行
//   - 流式中显示停止按钮
//   - 发送时：自动创建会话 → 设为当前 → 发送消息（用户无感）
//
// props:
//   - agentName:    当前 Agent 名称（用于问候语）
//   - modelName:    当前 Agent 模型名（用于底部小字信息）
//
// emits:
//   - send(content: string)  用户提交消息时（已自动创建会话）
//   - stop()                用户点击停止时

import { computed, nextTick, ref, watch, onMounted, useTemplateRef } from "vue";
import { SendHorizontal, Square } from "lucide-vue-next";
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import { useChatStore } from "../../stores/chat";
import { useToast } from "../../composables/useToast";
import { useAgentMeta, type AgentMeta } from "../../composables/useAgentMeta";
import AgentAvatar from "../common/AgentAvatar.vue";
import ImagePicker, { type ImageItem } from "./ImagePicker.vue";
import type { ContentBlock } from "../../types";

const props = defineProps<{
  agentName: string;
  modelName: string;
}>();

const emit = defineEmits<{
  send: [content: string, contentBlocks?: ContentBlock[]];
  stop: [];
}>();

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const toast = useToast();
const agentMeta = useAgentMeta();

// ============================================================================
// 状态
// ============================================================================

/** 文本草稿 */
const draft = ref<string>("");

// ============================================================================
// P2-2 多模态：待发送图片
// ============================================================================

/** 待发送图片列表（ImagePicker 受控） */
const pendingImages = ref<ImageItem[]>([]);

/** ImagePicker 的图片更新回调 */
function onImagesChange(next: ImageItem[]): void {
  pendingImages.value = next;
}

/** 提交中（防止双击重复创建会话） */
const submitting = ref<boolean>(false);

/** textarea DOM 引用（用于自动增高 + 自动聚焦） */
const textareaRef = useTemplateRef<HTMLTextAreaElement | null>("textareaRef");

// ============================================================================
// Agent 元数据（头像 + 问候语 + 推荐词）
// ============================================================================

/** 当前 Agent 的完整 meta */
const meta = computed<AgentMeta | null>(() => {
  const agent = agentsStore.current;
  if (!agent) return null;
  return agentMeta.getFullMeta(agent);
});

/** 个性化问候语 */
const greeting = computed<string>(() => {
  const desc = meta.value?.description;
  if (desc) {
    return `我是你的${desc}，有什么可以帮你？`;
  }
  if (props.agentName) {
    return `来和 ${props.agentName} 聊聊吧`;
  }
  return "有什么可以帮你的？";
});

// ============================================================================
// 提示词芯片
// ============================================================================

/** 推荐词优先级：meta.promptChips > system_prompt 关键词 > model 匹配 */
const promptChips = computed<string[]>(() => {
  // 1. 优先使用 meta.promptChips（模板自带）
  const chips = meta.value?.promptChips;
  if (chips && chips.length > 0) return chips;

  // 2. 最终降级：根据 model 名称匹配（meta 不存在时）
  const model = props.modelName.toLowerCase();
  if (model.includes("code") || model.includes("deepseek") || model.includes("gpt-4")) {
    return [
      "帮我写一段 Python 脚本，实现批量重命名",
      "解释这段代码的复杂度",
      "推荐一个项目的目录结构",
    ];
  }
  if (model.includes("claude") || model.includes("opus")) {
    return [
      "帮我润色一下这段文字，让它更专业",
      "解释一下量子纠缠的概念",
      "用一段话总结《三体》第一部的核心",
    ];
  }
  return [
    "用一段话总结《三体》第一部的核心",
    "帮我写一份周报模板",
    "推荐一本适合周末读的小说",
    "解释一下 Transformer 的注意力机制",
  ];
});

// ============================================================================
// 自动增高（与 ChatInput 保持一致）
// ============================================================================

const LINE_HEIGHT_PX = 24;
const MAX_ROWS = 8;
const VERTICAL_PADDING_PX = 22;
const maxHeightPx = LINE_HEIGHT_PX * MAX_ROWS + VERTICAL_PADDING_PX;
const heightPx = ref<number>(LINE_HEIGHT_PX + VERTICAL_PADDING_PX);

function autosize(): void {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = `${LINE_HEIGHT_PX + VERTICAL_PADDING_PX}px`;
  const sh = el.scrollHeight;
  const h = Math.min(sh, maxHeightPx);
  el.style.height = `${h}px`;
  heightPx.value = h;
}

watch(draft, () => {
  void nextTick(autosize);
  // 用户编辑消息 → 清空已应用模板
  if (chatStore.appliedTemplate) {
    chatStore.setAppliedTemplate(null);
  }
});

// ============================================================================
// 流式状态：复用 chatStore
// ============================================================================

const isStreaming = computed<boolean>(() => chatStore.isStreaming);

/** 实际是否禁用输入（流中或提交中时） */
const inputDisabled = computed<boolean>(
  () => submitting.value || props.agentName.length === 0 || isStreaming.value,
);

/** 发送按钮是否禁用（无文本+图片 + 其他禁用条件） */
const sendDisabled = computed<boolean>(
  () =>
    inputDisabled.value ||
    (draft.value.trim().length === 0 && pendingImages.value.length === 0),
);

// ============================================================================
// 自动聚焦
// ============================================================================

onMounted(() => {
  void nextTick(() => {
    textareaRef.value?.focus();
  });
});

// ============================================================================
// 芯片点击：填入但不发送
// ============================================================================

function onChipClick(text: string): void {
  if (inputDisabled.value) return;
  draft.value = text;
  void nextTick(() => {
    autosize();
    textareaRef.value?.focus();
    const el = textareaRef.value;
    if (el) {
      const len = el.value.length;
      el.setSelectionRange(len, len);
    }
  });
}

// ============================================================================
// 提交（Enter）
// ============================================================================

async function handleSend(): Promise<void> {
  if (inputDisabled.value) return;
  const trimmed = draft.value.trim();
  const hasImages = pendingImages.value.length > 0;
  // 必须提供文本或图片二者之一
  if (!trimmed && !hasImages) return;
  if (!agentsStore.currentId) {
    toast.error("当前没有可用 Agent");
    return;
  }

  // P2-2 多模态：有图片则构造 content_blocks
  let contentBlocks: ContentBlock[] | undefined;
  if (hasImages) {
    contentBlocks = [];
    if (trimmed) contentBlocks.push({ type: "text", text: trimmed });
    for (const img of pendingImages.value) {
      contentBlocks.push({ type: "image", data: img.data, media_type: img.media_type });
    }
  }

  submitting.value = true;
  try {
    if (!conversationsStore.currentId) {
      await conversationsStore.create(agentsStore.currentId);
    }
    const convId = conversationsStore.currentId;
    if (!convId) {
      toast.error("创建会话失败");
      return;
    }
    draft.value = "";
    pendingImages.value = [];
    void nextTick(autosize);
    chatStore.setAppliedTemplate(null);
    emit("send", trimmed, contentBlocks);
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    toast.error(`创建会话失败：${msg}`);
  } finally {
    submitting.value = false;
  }
}

function onKeydown(e: KeyboardEvent): void {
  if (e.key !== "Enter") return;
  if (e.shiftKey || e.ctrlKey || e.metaKey) return;
  e.preventDefault();
  void handleSend();
}

function onSendClick(): void {
  void handleSend();
}

function onStopClick(): void {
  emit("stop");
}
</script>

<template>
  <div class="welcome-root">
    <!-- 顶部：Agent 头像 + 个性化问候语 -->
    <div class="welcome-header">
      <AgentAvatar
        v-if="meta"
        :meta="meta"
        :size="64"
        class="welcome-avatar"
        aria-hidden="true"
      />
      <h2 class="welcome-title">{{ greeting }}</h2>
      <p class="welcome-subtitle">{{ agentName }} · {{ modelName }}</p>
    </div>

    <!-- 提示词芯片 -->
    <div class="welcome-chips">
      <button
        v-for="(chip, idx) in promptChips"
        :key="idx"
        type="button"
        class="chip"
        :disabled="inputDisabled"
        @click="onChipClick(chip)"
      >
        {{ chip }}
      </button>
    </div>


    <div
      :class="[
        'welcome-input',
        { 'welcome-input-streaming': isStreaming, 'welcome-input-disabled': inputDisabled },
      ]"
    >
      <!-- P2-2 多模态：图片选择器 -->
      <div class="welcome-image-picker">
        <ImagePicker
          :images="pendingImages"
          :disabled="inputDisabled"
          @update:images="onImagesChange"
        />
      </div>
      <textarea
        ref="textareaRef"
        v-model="draft"
        class="welcome-textarea"
        :placeholder="isStreaming ? '生成中...' : '输入消息，Enter 发送，Shift+Enter 换行'"
        :disabled="inputDisabled"
        rows="1"
        :style="{ height: `${heightPx}px` }"
        :maxlength="20000"
        @keydown="onKeydown"
      />
      <div class="welcome-toolbar">
        <span class="welcome-hint">Enter 发送 · Shift+Enter 换行</span>
        <button
          v-if="isStreaming"
          type="button"
          class="welcome-btn welcome-btn-stop"
          aria-label="停止生成"
          @click="onStopClick"
        >
          <Square :size="14" aria-hidden="true" />
          <span>停止</span>
        </button>
        <button
          v-else
          type="button"
          class="welcome-btn welcome-btn-send"
          :disabled="sendDisabled"
          aria-label="发送消息"
          @click="onSendClick"
        >
          <SendHorizontal :size="16" aria-hidden="true" />
          <span>发送</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.welcome-root {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px var(--ip-spacing-6);
  gap: var(--ip-spacing-6);
  background: var(--ip-color-bg-primary);
  overflow-y: auto;
}

/* ===== 头部：头像 + 问候 ===== */
.welcome-header {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-3);
}

.welcome-avatar {
  /* AgentAvatar 组件自带 box-shadow，此处添加外发光效果 */
  filter: drop-shadow(0 4px 12px rgba(0, 0, 0, 0.08));
}

.welcome-title {
  margin: 0;
  font-size: var(--ip-text-h2-size, 24px);
  font-weight: var(--ip-font-weight-semibold, 600);
  color: var(--ip-color-text-primary);
  letter-spacing: -0.01em;
}

.welcome-subtitle {
  margin: 0;
  font-size: var(--ip-text-body-sm-size, 13px);
  color: var(--ip-color-text-tertiary);
}

/* ===== 提示词芯片 ===== */
.welcome-chips {
  display: flex;
  flex-wrap: wrap;
  justify-content: center;
  gap: var(--ip-spacing-2);
  max-width: 640px;
  width: 100%;
}

/* P2-2 多模态：图片选择器区域（缩略图列表与 textarea 间距） */
.welcome-image-picker {
  display: flex;
  width: 100%;
  padding: 0 2px 8px;
  box-sizing: border-box;
}

.chip {
  appearance: none;
  padding: 8px 14px;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-full, 9999px);
  background: var(--ip-color-bg-secondary);
  color: var(--ip-color-text-secondary);
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  line-height: 1.4;
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.chip:hover:not(:disabled) {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-primary);
}

.chip:active:not(:disabled) {
  transform: translateY(0.5px);
}

.chip:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.chip:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* ===== 输入框 ===== */
.welcome-input {
  width: 100%;
  max-width: 640px;
  background: var(--ip-color-bg-elevated);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-lg, 12px);
  box-shadow: 0 4px 16px -8px rgba(0, 0, 0, 0.08);
  transition:
    border-color var(--ip-duration-base, 200ms) var(--ip-ease-out),
    box-shadow var(--ip-duration-base, 200ms) var(--ip-ease-out);
  display: flex;
  flex-direction: column;
}

.welcome-input:focus-within {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.welcome-input-streaming {
  border-color: var(--ip-color-border-default);
}

.welcome-input-disabled {
  opacity: 0.7;
}

.welcome-textarea {
  display: block;
  width: 100%;
  resize: none;
  border: none;
  outline: none;
  background: transparent;
  padding: 14px 16px;
  font-family: inherit;
  font-size: var(--ip-text-body-size, 15px);
  line-height: var(--ip-text-body-lh, 1.6);
  color: var(--ip-color-text-primary);
  box-sizing: border-box;
  overflow-y: auto;
}

.welcome-textarea::placeholder {
  color: var(--ip-color-text-tertiary);
}

.welcome-textarea:disabled {
  background: transparent;
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
}

.welcome-toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px 10px;
  border-top: 1px solid var(--ip-color-border-default);
}

.welcome-hint {
  font-size: var(--ip-text-caption-size, 12px);
  color: var(--ip-color-text-tertiary);
}

.welcome-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  appearance: none;
  height: 32px;
  padding: 0 14px;
  font-family: inherit;
  font-size: var(--ip-text-body-sm-size, 13px);
  font-weight: var(--ip-font-weight-medium, 500);
  border-radius: var(--ip-radius-md, 8px);
  border: 1px solid transparent;
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.welcome-btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.welcome-btn:active:not(:disabled) {
  transform: translateY(0.5px);
}

.welcome-btn-send {
  background: var(--ip-primary-600);
  color: var(--ip-color-text-on-primary);
  border-color: var(--ip-primary-600);
}

.welcome-btn-send:hover:not(:disabled) {
  background: var(--ip-primary-700);
  border-color: var(--ip-primary-700);
}

.welcome-btn-send:disabled {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  border-color: var(--ip-color-bg-tertiary);
  cursor: not-allowed;
}

.welcome-btn-stop {
  background: var(--ip-danger-base);
  color: var(--ip-color-text-on-danger);
  border-color: var(--ip-danger-base);
}

.welcome-btn-stop:hover {
  background: var(--ip-danger-hover);
  border-color: var(--ip-danger-hover);
}
</style>

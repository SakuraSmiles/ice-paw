<script setup lang="ts">
// 首页居中输入框组件（WelcomeScreen 专用）
//
// 职责：
//   - 居中显示一个大尺寸的多行输入框
//   - 顶部显示 Agent 头像 + 「有什么可以帮你的？」问候语
//   - 下方展示可点击的提示词芯片，点击后填入输入框（不自动发送）
//   - Enter 发送；Shift+Enter 换行（沿用 ChatInput 的键位语义）
//   - 流式中显示停止按钮（沿用 ChatInput 的视觉规范）
//   - 发送时：自动创建会话 → 设为当前 → 发送消息（用户无感）
//
// props:
//   - agentName:    当前 Agent 名称（用于问候语）
//   - modelName:    当前 Agent 模型名（用于底部小字信息）
//
// emits:
//   - send(content: string)  用户提交消息时（已自动创建会话）
//   - stop()                用户点击停止时

import { computed, nextTick, ref, watch } from "vue";
import { SendHorizontal, Square } from "lucide-vue-next";
import { useAgentsStore } from "../../stores/agents";
import { useConversationsStore } from "../../stores/conversations";
import { useChatStore } from "../../stores/chat";
import { useToast } from "../../composables/useToast";
import pawLogo from "../../assets/logo/paw.svg";

const props = defineProps<{
  agentName: string;
  modelName: string;
}>();

const emit = defineEmits<{
  send: [content: string];
  stop: [];
}>();

const agentsStore = useAgentsStore();
const conversationsStore = useConversationsStore();
const chatStore = useChatStore();
const toast = useToast();

// ============================================================================
// 状态
// ============================================================================

/** 文本草稿 */
const draft = ref<string>("");

/** 提交中（防止双击重复创建会话） */
const submitting = ref<boolean>(false);

/** textarea DOM 引用（用于自动增高 + 自动聚焦） */
const textareaRef = ref<HTMLTextAreaElement | null>(null);

// ============================================================================
// 提示词芯片
// ============================================================================

/** 根据 Agent 模型动态生成的提示词芯片 */
const promptChips = computed<string[]>(() => {
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
  // 默认中文提示词
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
});

// ============================================================================
// 流式状态：复用 chatStore
// ============================================================================

const isStreaming = computed<boolean>(() => chatStore.isStreaming);

/** 实际是否禁用输入（流中或提交中时） */
const inputDisabled = computed<boolean>(
  () => submitting.value || props.agentName.length === 0 || isStreaming.value,
);

/** 发送按钮是否禁用（无内容 + 其他禁用条件） */
const sendDisabled = computed<boolean>(
  () => inputDisabled.value || draft.value.trim().length === 0,
);

// ============================================================================
// 自动聚焦
// ============================================================================

onMountedFocus();

function onMountedFocus(): void {
  void nextTick(() => {
    textareaRef.value?.focus();
  });
}

// ============================================================================
// 芯片点击：填入但不发送
// ============================================================================

function onChipClick(text: string): void {
  if (inputDisabled.value) return;
  draft.value = text;
  void nextTick(() => {
    autosize();
    textareaRef.value?.focus();
    // 移动光标到末尾
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
  if (!trimmed) return;
  if (!agentsStore.currentId) {
    toast.error("当前没有可用 Agent");
    return;
  }

  submitting.value = true;
  try {
    // 1. 若还没有会话，自动创建（首条消息即创建会话）
    if (!conversationsStore.currentId) {
      await conversationsStore.create(agentsStore.currentId);
    }
    const convId = conversationsStore.currentId;
    if (!convId) {
      toast.error("创建会话失败");
      return;
    }
    // 2. 清空草稿
    draft.value = "";
    void nextTick(autosize);
    // 3. 触发外层（ChatPage）切换到正常聊天界面 + 加载历史
    emit("send", trimmed);
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
    <!-- 顶部：头像 + 问候语 -->
    <div class="welcome-header">
      <div class="welcome-avatar" aria-hidden="true">
        <img :src="pawLogo" alt="" class="welcome-avatar-img" />
      </div>
      <h2 class="welcome-title">有什么可以帮你的？</h2>
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

    <!-- 居中输入框 -->
    <div
      :class="[
        'welcome-input',
        { 'welcome-input-streaming': isStreaming, 'welcome-input-disabled': inputDisabled },
      ]"
    >
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
  width: 64px;
  height: 64px;
  border-radius: var(--ip-radius-full);
  background: var(--ip-primary-500);
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 6px 20px -8px rgba(59, 130, 246, 0.5);
}

.welcome-avatar-img {
  width: 32px;
  height: 32px;
  color: #fff;
  filter: brightness(0) invert(1);
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
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out),
    border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out),
    color var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out, ease-out);
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
  box-shadow: var(--ip-shadow-focus, 0 0 0 3px rgba(59, 130, 246, 0.3));
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
    border-color var(--ip-duration-base, 200ms) var(--ip-ease-out, ease-out),
    box-shadow var(--ip-duration-base, 200ms) var(--ip-ease-out, ease-out);
  display: flex;
  flex-direction: column;
}

.welcome-input:focus-within {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus, 0 0 0 3px rgba(59, 130, 246, 0.25));
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
  transition: var(--ip-transition-colors, all 150ms ease-out);
}

.welcome-btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus, 0 0 0 3px rgba(59, 130, 246, 0.3));
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
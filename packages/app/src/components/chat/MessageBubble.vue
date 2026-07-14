<script setup lang="ts">
// 单条消息气泡
//
// 职责：
//   - 按消息 role 渲染不同样式：用户（右）、助手（左）、系统（居中灰字）
//   - 助手消息正文用 MarkdownContent 渲染（标题 / 粗体 / 代码块等）
//   - 用户消息保持纯文本 + white-space: pre-wrap（保留换行）
//   - 流式中的助手消息尾部追加光标闪烁动画（::after 伪元素）
//   - 错误态：红色边框 + 错误文本 + 「重试」按钮（仅助手；重试功能 P1，目前仅占位）
//
// props:
//   - message:        Message 实体
//   - isStreaming:    是否为正在流式生成的那条（用于显示光标）
//   - renderMarkdown: 是否渲染 Markdown。流式中传 false 走纯文本路径以减轻性能压力；
//                     流式结束后由父组件切到 true，触发完整 Markdown 渲染。
//   - prevRole:       上一条消息的角色（用于跨 agent 间距；null 表示这是首条）
//
// emits:
//   - retry: 点击重试按钮时触发

import { computed, onUnmounted, ref } from "vue";
import type { Message, MessageRole } from "../../types";
import MarkdownContent from "./MarkdownContent.vue";
import ToolCallBlock from "./ToolCallBlock.vue";
import ThinkingBlock from "./ThinkingBlock.vue";
import { useChatStore } from "../../stores/chat";

const props = defineProps<{
  message: Message;
  isStreaming: boolean;
  /** P2-3: Token usage data (from chat:done event) */
  usage?: {
    prompt_tokens: number;
    completion_tokens: number;
    cached_tokens: number;
  } | null;
  /**
   * 是否渲染 Markdown。
   * - 流式中（message.id === streamingId）传 false：直接展示原文 + 光标，
   *   跳过 markdown-it 解析 + highlight.js 高亮，避免每个 chunk 都跑一次完整渲染。
   * - 流式结束后传 true：切到 MarkdownContent 渲染完整 Markdown。
   */
  renderMarkdown: boolean;
  /** 上一条消息的角色；null 表示这是列表里的第一条 */
  prevRole?: MessageRole | null;
  /** 是否正在重试中（LLM 流式中断后自动重试） */
  isRetrying?: boolean;
  /** 重试进度文本，如 "2/4" */
  retryProgress?: string;
  /** P2-1: 实时活跃的工具调用（仅流式中的助手消息需要） */
  activeToolCalls?: Array<{
    id: string;
    name: string;
    argumentsBuffer: string;
    ended: boolean;
  }>;
  /** P2-1: 实时思考过程内容（仅流式中的助手消息需要） */
  thinkingContent?: string;
}>();

const emit = defineEmits<{
  retry: [message: Message];
}>();

/** 当前消息角色快捷判断 */
const isAssistant = computed<boolean>(() => props.message.role === "assistant");
const isUser = computed<boolean>(() => props.message.role === "user");

/** P2-1: 从 content_blocks 解析出的工具调用列表 */
interface ParsedToolCall {
  id: string;
  name: string;
  input: string;
  result?: string;
  isError?: boolean;
}

/** P2-2: 图片块预处理成预览 data URL（含 media_type 前缀） */
interface ParsedImage {
  /** 原始 base64 (无前缀) */
  data: string;
  media_type: string;
  /** data URL，供 <img src> / lightbox 预览 */
  src: string;
}

/** 把 base64 + media_type 转成完整 data URL（结果受 const 缓存影响，这里用函数返回新值） */
function buildImageSrc(data: string, mediaType: string): string {
  return `data:${mediaType};base64,${data}`;
}

/** 解析 content_blocks JSON */
const parsedBlocks = computed(() => {
  if (!props.message.content_blocks || props.message.content_blocks === "[]") {
    return {
      toolCalls: [] as ParsedToolCall[],
      thinking: "",
      images: [] as ParsedImage[],
    };
  }
  try {
    const blocks = JSON.parse(props.message.content_blocks);

    // 第一遍：收集 tool_result，建立 id → result 映射
    const resultMap: Record<string, { content: string; isError: boolean }> = {};
    for (const block of blocks) {
      if (block.type === "tool_result") {
        resultMap[block.tool_use_id] = {
          content: block.content,
          isError: block.is_error ?? false,
        };
      }
    }

    // 第二遍：构建 toolCalls，关联对应的 result；同步提取 images 与 thinking
    const toolCalls: ParsedToolCall[] = [];
    let thinking = "";
    const images: ParsedImage[] = [];
    for (const block of blocks) {
      if (block.type === "tool_use") {
        const result = resultMap[block.id];
        toolCalls.push({
          id: block.id,
          name: block.name,
          input: block.input,
          result: result?.content,
          isError: result?.isError,
        });
      } else if (block.type === "thinking") {
        thinking += block.thinking || "";
      } else if (block.type === "image") {
        const data = block.data;
        const mediaType = block.media_type;
        if (typeof data === "string" && data.length > 0 && typeof mediaType === "string") {
          images.push({
            data,
            media_type: mediaType,
            src: buildImageSrc(data, mediaType),
          });
        }
      }
    }
    return { toolCalls, thinking, images };
  } catch {
    return {
      toolCalls: [] as ParsedToolCall[],
      thinking: "",
      images: [] as ParsedImage[],
    };
  }
});

/** 当前活跃的工具调用（从 store 传入，用于实时展示） */
const activeToolCalls = computed(() => props.activeToolCalls || []);
const thinkingContent = computed(() => props.thinkingContent || "");

/** P2-1: 聊天 store（用于实时 ToolCallBlock 读取 toolResults） */
const chatStore = useChatStore();

/** 是否展示光标（仅流式中的助手消息、无错误） */
const showCursor = computed<boolean>(
  () => props.isStreaming && !props.message.error && isAssistant.value,
);

/** 是否为跨 agent（上一条存在且角色不同） */
const isCrossAgent = computed<boolean>(
  () => props.prevRole != null && props.prevRole !== props.message.role,
);

/**
 * 顶部间距：跨 agent 用更大间距，连续同 agent 用更紧凑间距。
 * token 来自 @ice-paw/ui 的 design system；main.ts 已 import '@ice-paw/ui/styles'，
 * --ip-* 变量在 :root 已注入，可直接引用无需 fallback。
 */
const marginTop = computed<string>(() =>
  isCrossAgent.value
    ? "var(--ip-message-gap-cross, 16px)"
    : "var(--ip-message-gap-same, 4px)",
);

/** 重试按钮点击 */
function onRetry(): void {
  emit("retry", props.message);
}

// ============================================================================
// P2-2 多模态：图片 Lightbox 状态
// ============================================================================

/** 当前展开的全屏图片 src（null=关闭） */
const lightboxSrc = ref<string | null>(null);
/** 用于键盘事件控制 Esc 关闭 */
const lightboxAlt = ref<string>("");

/** 点击缩略图 → 打开 lightbox */
function openLightbox(src: string, alt: string): void {
  lightboxSrc.value = src;
  lightboxAlt.value = alt;
  // a11y: 锁定 body 滚动（轻量：不处理 tabindex trap，因 Lightbox 通常在静态浏览场景下使用）
  if (typeof document !== "undefined") {
    const prev = document.body.style.overflow;
    document.body.dataset.prevOverflow = prev;
    document.body.style.overflow = "hidden";
  }
  if (typeof window !== "undefined") {
    window.addEventListener("keydown", onLightboxKey);
  }
}

/** 关闭 lightbox */
function closeLightbox(): void {
  lightboxSrc.value = null;
  lightboxAlt.value = "";
  if (typeof document !== "undefined") {
    const prev = document.body.dataset.prevOverflow ?? "";
    document.body.style.overflow = prev;
    delete document.body.dataset.prevOverflow;
  }
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", onLightboxKey);
  }
}

/** Esc 关闭 */
function onLightboxKey(e: KeyboardEvent): void {
  if (e.key === "Escape") {
    e.preventDefault();
    closeLightbox();
  }
}

/** 组件销毁时清理全局监听 & 恢复 body */
onUnmounted(() => {
  if (typeof window !== "undefined") {
    window.removeEventListener("keydown", onLightboxKey);
  }
  if (typeof document !== "undefined" && lightboxSrc.value !== null) {
    const prev = document.body.dataset.prevOverflow ?? "";
    document.body.style.overflow = prev;
    delete document.body.dataset.prevOverflow;
  }
});
</script>

<template>
  <!--
    列表级 v-memo（作用在外层 bubble-row）：
      - 依赖项：message.content / message.content_blocks / message.role / renderMarkdown。
      - 历史消息稳定 → deps 不变 → Vue 跳过整个子树 patch，
        避免列表被 stream 增量刷新时把整列历史气泡也连带 patch 一次。
      - 流式中：deps 每 chunk 都变（content 递增），v-memo 正确触发更新，
        不会卡死。配合 MessageList 流式期间不传 renderMarkdown=true，
        单 chunk 成本仅 1 个文本节点 + v-memo 短路检查。
      - P2-2：加 content_blocks 作为依赖项，多模态图片上传后不会因 v-memo
        短路而丢失渲染。
  -->
  <!-- P2-2: 图片 Lightbox（全屏放大查看，点击遮罩或 Esc 关闭） -->
  <Teleport to="body">
    <div
      v-if="lightboxSrc"
      class="bubble-image-lightbox"
      role="dialog"
      aria-modal="true"
      :aria-label="lightboxAlt"
      @click.self="closeLightbox"
    >
      <button
        type="button"
        class="bubble-image-lightbox-close"
        aria-label="关闭预览"
        title="关闭（Esc）"
        @click="closeLightbox"
      >
        ✕
      </button>
      <img :src="lightboxSrc" :alt="lightboxAlt" class="bubble-image-lightbox-img" />
    </div>
  </Teleport>

  <div
    v-memo="[message.content, message.content_blocks, message.role, renderMarkdown]"
    :class="['bubble-row', `bubble-row-${message.role}`]"
    :style="{ marginTop }"
  >
    <!-- 系统消息：居中灰字 -->
    <div v-if="message.role === 'system'" class="bubble-system">
      <span class="bubble-system-text">{{ message.content }}</span>
    </div>

    <!-- 用户消息：右侧气泡，纯文本（保持原始换行与空格） -->
    <div v-else-if="isUser" class="bubble-user">
      <div class="bubble-content">
        <!-- P2-2 多模态：图片缩略图数组（点击打开 lightbox） -->
        <div
          v-if="parsedBlocks.images.length > 0"
          class="bubble-image-grid"
          role="list"
          :aria-label="`附件 ${parsedBlocks.images.length} 张图片`"
        >
          <button
            v-for="(img, idx) in parsedBlocks.images"
            :key="`u-img-${idx}`"
            type="button"
            class="bubble-image-thumb"
            :title="`查看第 ${idx + 1} 张`"
            :aria-label="`查看附件第 ${idx + 1} 张`"
            @click="openLightbox(img.src, `附件 ${idx + 1}`)"
          >
            <img :src="img.src" :alt="`附件 ${idx + 1}`" />
          </button>
        </div>
        <div v-if="message.content" class="bubble-text">
          <div class="bubble-text-content">{{ message.content }}</div>
        </div>
        <div v-if="message.error" class="bubble-error">
          <span class="error-text">{{ message.error }}</span>
        </div>
      </div>
    </div>

    <!--
      助手消息：左侧气泡。
      流式渲染策略：
        - 流式中（renderMarkdown=false）→ 直接渲染纯文本 + 光标，跳过 markdown-it
        - 流式结束（renderMarkdown=true） → 切到 MarkdownContent 完整渲染
      切换由父组件控制：流式开始传 false，每 chunk 期间保持 false；
      流式完成（chat:done）那一刻翻转成 true，触发 MarkdownContent 接管。
    -->
    <div v-else class="bubble-assistant">
      <div class="bubble-content">
        <!-- P2-1: 思考过程（仅 Anthropic 模型） -->
        <ThinkingBlock
          v-if="thinkingContent || (!isStreaming && parsedBlocks.thinking)"
          :content="isStreaming ? thinkingContent : parsedBlocks.thinking"
          :streaming="isStreaming"
        />

        <!-- P2-1: 历史消息中的工具调用（从 content_blocks 解析） -->
        <ToolCallBlock
          v-for="tc in parsedBlocks.toolCalls"
          :key="tc.id"
          :name="tc.name"
          :arguments="tc.input"
          :ended="true"
          :result="tc.result"
          :is-error="tc.isError"
        />

        <!-- P2-1: 实时工具调用（流式中） -->
        <ToolCallBlock
          v-for="tc in activeToolCalls"
          :key="'live-' + tc.id"
          :name="tc.name"
          :arguments="tc.argumentsBuffer"
          :ended="tc.ended"
          :result="chatStore.toolResults[tc.id]?.content"
          :is-error="chatStore.toolResults[tc.id]?.isError"
        />

        <!-- P2-2 多模态：历史消息中的图片缩略图数组 -->
        <div
          v-if="!isStreaming && parsedBlocks.images.length > 0"
          class="bubble-image-grid"
          role="list"
          :aria-label="`附件 ${parsedBlocks.images.length} 张图片`"
        >
          <button
            v-for="(img, idx) in parsedBlocks.images"
            :key="`a-img-${idx}`"
            type="button"
            class="bubble-image-thumb"
            :title="`查看第 ${idx + 1} 张`"
            :aria-label="`查看附件第 ${idx + 1} 张`"
            @click="openLightbox(img.src, `附件 ${idx + 1}`)"
          >
            <img :src="img.src" :alt="`附件 ${idx + 1}`" />
          </button>
        </div>

        <div
          v-if="message.content || isStreaming"
          class="bubble-text"
          :class="{ 'bubble-text-streaming': showCursor }"
        >
          <MarkdownContent
            v-if="renderMarkdown"
            :content="message.content"
          />
          <div v-else class="bubble-text-content">{{ message.content }}</div>
        </div>
        <div v-if="message.error" class="bubble-error">
          <span class="error-text">{{ message.error }}</span>
          <button class="btn-retry" type="button" @click="onRetry">重试</button>
        </div>
        <div v-else-if="isRetrying" class="bubble-retrying">
          <span class="retrying-indicator" />正在重新连接... {{ retryProgress }}
        </div>
        <!-- P2-3: Token 用量（仅流式结束后显示在助手消息底部） -->
        <div
          v-if="!isStreaming && usage"
          class="bubble-usage"
        >
          <span class="usage-label">Tokens</span>
          <span class="usage-item">
            <span class="usage-key">Prompt</span>
            <span class="usage-val">{{ usage.prompt_tokens }}</span>
          </span>
          <span v-if="usage.cached_tokens > 0" class="usage-item usage-cached">
            <span class="usage-key">Cached</span>
            <span class="usage-val">{{ usage.cached_tokens }}</span>
          </span>
          <span class="usage-item">
            <span class="usage-key">Completion</span>
            <span class="usage-val">{{ usage.completion_tokens }}</span>
          </span>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.bubble-row {
  display: flex;
  width: 100%;
  padding: 0 20px;
  box-sizing: border-box;
}

/* 用户消息：右对齐 */
.bubble-row-user {
  justify-content: flex-end;
}

/* 助手消息：左对齐 */
.bubble-row-assistant {
  justify-content: flex-start;
}

/* 系统消息：居中 */
.bubble-row-system {
  justify-content: center;
}

.bubble-content {
  max-width: var(--ip-message-max-w, min(720px, 80%));
  display: flex;
  flex-direction: column;
  gap: 6px;
  min-width: 0;
}

.bubble-text {
  padding: 10px 14px;
  border-radius: 10px;
  font-size: 14px;
  line-height: 1.55;
  word-break: break-word;
  overflow-wrap: anywhere;
  position: relative;
  /* flex column 让流式光标（::after 伪元素）自然落在 markdown 末尾换行处 */
  display: flex;
  flex-direction: column;
  gap: 2px;
}

.bubble-text-content {
  /* 用户消息：保留原始换行与空格 */
  white-space: pre-wrap;
  word-break: break-word;
}

/* 用户气泡：主题色蓝底白字 */
.bubble-user .bubble-text {
  background: var(--ip-color-bg-user-bubble);     /* #2563EB v1.0.3 */
  color: var(--ip-color-text-on-user-bubble);    /* #FFFFFF v1.0.3 */
  border-bottom-right-radius: 4px;
}
/* hover 微亮：filter brightness 让蓝色更明显 */
.bubble-user:hover .bubble-text {
  filter: brightness(1.08);
}

/* 助手气泡：浅灰底深字（v1.0.3：与用户气泡形成强视觉分层） */
.bubble-assistant .bubble-text {
  background: var(--ip-color-bg-ai-bubble);      /* #F5F5F5 v1.0.3 */
  color: var(--ip-color-text-primary);
  border-bottom-left-radius: 4px;
}

/* 系统气泡 */
.bubble-system {
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 4px 12px;
}

.bubble-system-text {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  background: var(--ip-color-bg-tertiary);
  padding: 4px 12px;
  border-radius: 10px;
}

/* 流式光标：仅助手消息、正在流式生成、且无错误时出现
   用 ::after 伪元素附加在 bubble-text 末尾，不污染 markdown 渲染结果 */
.bubble-text-streaming::after {
  content: "";
  display: inline-block;
  width: 7px;
  height: 14px;
  margin-top: 2px;
  align-self: flex-start;
  background: var(--cursor-fg, currentColor);
  animation: cursor-blink 1s steps(1) infinite;
  border-radius: 1px;
  /* a11y: 屏幕阅读器忽略装饰性光标 */
  speak: none;
}

@keyframes cursor-blink {
  0%,
  50% {
    opacity: 1;
  }
  50.01%,
  100% {
    opacity: 0;
  }
}

/* 减少动效偏好：光标常亮不闪烁，避免对前庭敏感用户造成干扰 */
@media (prefers-reduced-motion: reduce) {
  .bubble-text-streaming::after {
    animation: none;
    opacity: 1;
  }
}

/* 错误条 */
.bubble-error {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 6px 12px;
  font-size: var(--ip-text-caption-size);
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-md);
  background: var(--ip-danger-bg);
  color: var(--ip-danger-text);
}

.error-text {
  flex: 1;
  word-break: break-word;
}

.btn-retry {
  flex-shrink: 0;
  padding: 2px 10px;
  font-size: var(--ip-text-caption-size);
  border: 1px solid var(--ip-danger-border);
  border-radius: var(--ip-radius-sm);
  background: transparent;
  color: var(--ip-danger-text);
  cursor: pointer;
  font-family: inherit;
  transition: background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.btn-retry:hover {
  background: var(--ip-danger-bg);
}

/* 重试指示器 */
.bubble-retrying {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 12px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.retrying-indicator {
  display: inline-block;
  width: 12px;
  height: 12px;
  border: 2px solid var(--ip-color-text-tertiary);
  border-top-color: transparent;
  border-radius: 50%;
  animation: retrying-spin 1s linear infinite;
}

@keyframes retrying-spin {
  to {
    transform: rotate(360deg);
  }
}

@media (prefers-reduced-motion: reduce) {
  .retrying-indicator {
    animation: none;
  }
}

/* P2-3: Token usage display */
.bubble-usage {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  margin-top: var(--ip-spacing-1);
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  font-family: var(--ip-font-mono, monospace);
}

.usage-label {
  font-size: 10px;
  text-transform: uppercase;
  letter-spacing: 0.5px;
  color: var(--ip-color-text-quaternary);
  margin-right: 2px;
}

.usage-item {
  display: flex;
  gap: 3px;
  align-items: baseline;
}

.usage-key {
  color: var(--ip-color-text-quaternary);
}

.usage-val {
  color: var(--ip-color-text-tertiary);
}

.usage-cached .usage-val {
  color: var(--ip-primary-500);
}

/* ===== P2-2 多模态：图片缩略图阵列 ===== */
.bubble-image-grid {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ip-spacing-2);
}

.bubble-image-thumb {
  appearance: none;
  display: block;
  padding: 0;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-tertiary);
  cursor: zoom-in;
  overflow: hidden;
  width: 96px;
  height: 96px;
  flex-shrink: 0;
  transition:
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    border-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.bubble-image-thumb img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
  max-height: 100%;
}

.bubble-image-thumb:hover {
  border-color: var(--ip-color-border-strong);
  transform: translateY(-1px);
  box-shadow: 0 4px 12px -4px rgba(0, 0, 0, 0.18);
}

.bubble-image-thumb:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}
</style>

<!-- P2-2 lightbox: Teleport 到 body，需保留样式可见，故使用非 scoped 块 -->
<style>
.bubble-image-lightbox {
  position: fixed;
  inset: 0;
  z-index: 9999;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  background: rgba(0, 0, 0, 0.78);
  backdrop-filter: blur(4px);
  -webkit-backdrop-filter: blur(4px);
  cursor: zoom-out;
  animation: bubble-image-lightbox-in 150ms ease-out;
}

@keyframes bubble-image-lightbox-in {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}

.bubble-image-lightbox-img {
  display: block;
  max-width: min(92vw, 1600px);
  max-height: 92vh;
  object-fit: contain;
  border-radius: var(--ip-radius-md, 8px);
  box-shadow: 0 24px 64px -16px rgba(0, 0, 0, 0.6);
  cursor: auto;
  user-select: none;
}

.bubble-image-lightbox-close {
  position: absolute;
  top: 16px;
  right: 16px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 36px;
  height: 36px;
  padding: 0;
  border: 1px solid rgba(255, 255, 255, 0.4);
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.5);
  color: #fff;
  font-family: inherit;
  font-size: 18px;
  line-height: 1;
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.bubble-image-lightbox-close:hover {
  background: var(--ip-danger-base, #ef4444);
  border-color: var(--ip-danger-base, #ef4444);
}

.bubble-image-lightbox-close:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus, 0 0 0 2px rgba(59, 130, 246, 0.4));
}

@media (prefers-reduced-motion: reduce) {
  .bubble-image-lightbox {
    animation: none;
  }
}
</style>

<script setup lang="ts">
// 聊天输入框
//
// 职责：
//   - 多行 textarea，自动增高（最多 6 行高度）
//   - Enter 发送；Shift+Enter / Ctrl+Enter / Meta+Enter 换行
//   - 右下发送按钮：默认显示 SendHorizontal 图标；流式中切换为 Square + 红色高亮
//
//   - 图片附件：toolbar 左侧 Paperclip 按钮 + 中间 28px chip 行（最多 4 个 + "+N"）
//
//   - 全部样式走 --ip-input-* Design Token，
//     焦点态使用 --ip-shadow-focus 发光环
//   - 暗色模式：颜色由 token 自动覆盖
//
// props:
//   - disabled:   是否禁用整个输入（无会话时由外层控制）
//   - streaming:  是否正在流式生成
//
// emits:
//   - send(content: string, contentBlocks?: ContentBlock[])  发送时
//   - stop()                                                  停止生成

import { computed, nextTick, ref, watch, useTemplateRef } from "vue";
import { SendHorizontal, Square, Paperclip, X } from "lucide-vue-next";
import { IpPopconfirm } from "@ice-paw/ui";
import { useChatStore } from "../../stores/chat";
import { useAgentsStore } from "../../stores/agents";
import {
  ACCEPT_ATTR,
  useImageFiles,
  type ImageItem,
} from "../../composables/useImageFiles";
import type { ContentBlock } from "../../types";

const props = defineProps<{
  disabled: boolean;
  streaming: boolean;
}>();

const emit = defineEmits<{
  send: [content: string, contentBlocks?: ContentBlock[]];
  stop: [];
}>();

const chatStore = useChatStore();
const agentsStore = useAgentsStore();

// ============================================================================
// 文本草稿
// ============================================================================

const draft = ref<string>("");

// ============================================================================
// P2-2 多模态：待发送图片（拆分自原 ImagePicker.vue → 仅保留核心逻辑）
// ============================================================================

/** 待发送图片列表 */
const pendingImages = ref<ImageItem[]>([]);

/** 隐藏的 file input 引用 */
const fileInputRef = useTemplateRef<HTMLInputElement | null>("fileInputRef");

const { processFiles } = useImageFiles(
  () => pendingImages.value,
  (images) => {
    pendingImages.value = images;
  },
);

/** 当前 Agent 是否支持图片输入 */
const supportsVision = computed<boolean>(
  () => agentsStore.current?.supports_vision ?? false,
);

/** 附件按钮是否禁用：流式中 或 当前 Agent 不支持图片 */
const imagePickerDisabled = computed<boolean>(
  () => props.streaming || !supportsVision.value,
);

// 图片缩略图预览：超过上限的折叠进 +N 按钮
const MAX_VISIBLE_CHIPS = 4;
const visibleImages = computed<ImageItem[]>(() =>
  pendingImages.value.slice(0, MAX_VISIBLE_CHIPS),
);
const overflowCount = computed<number>(() =>
  Math.max(0, pendingImages.value.length - MAX_VISIBLE_CHIPS),
);

/** 溢出图片列表（超出 MAX_VISIBLE_CHIPS 的部分） */
const overflowImages = computed<ImageItem[]>(() =>
  pendingImages.value.slice(MAX_VISIBLE_CHIPS),
);

/** 溢出图片文件名列表，用于 title / Popconfirm description */
const overflowNamesText = computed<string>(() =>
  overflowImages.value
    .map((img, idx) => img.fileName ?? `图片 ${idx + 1}`)
    .join("\n"),
);

/** +N Popconfirm 显隐状态 */
const overflowPopoverOpen = ref<boolean>(false);

function triggerFilePicker(): void {
  if (imagePickerDisabled.value) return;
  fileInputRef.value?.click();
}

function onFileChange(e: Event): void {
  const target = e.target as HTMLInputElement | null;
  if (!target) return;
  const files = target.files;
  if (!files || files.length === 0) return;
  const fileList = Array.from(files);
  target.value = "";
  void processFiles(fileList);
}

function removeImage(idx: number): void {
  if (props.streaming) return;
  if (idx < 0 || idx >= pendingImages.value.length) return;
  const next = [...pendingImages.value];
  next.splice(idx, 1);
  pendingImages.value = next;
}

/** textarea paste 事件：检测剪贴板中的图片文件并添加 */
function onPaste(e: ClipboardEvent): void {
  const items = e.clipboardData?.items;
  if (!items) return;
  const imageFiles: File[] = [];
  for (const item of items) {
    if (item.type.startsWith("image/")) {
      const file = item.getAsFile();
      if (file) imageFiles.push(file);
    }
  }
  if (imageFiles.length > 0) {
    e.preventDefault();
    void processFiles(imageFiles);
  }
}

// ============================================================================
// Textarea: 自动增高
// ============================================================================

const textareaRef = useTemplateRef<HTMLTextAreaElement | null>("textareaRef");

/** 单行高度（与 line-height 一致） */
const LINE_HEIGHT_PX = 22;

/** 最大行数（含 padding 折算的余量） */
const MAX_ROWS = 6;

/** textarea 上下 padding 之和 */
const VERTICAL_PADDING_PX = 18;

/** textarea 的 maxHeight（px），用于自动增高封顶 */
const maxHeightPx = LINE_HEIGHT_PX * MAX_ROWS + VERTICAL_PADDING_PX;

/** textarea 的高度（px），由行数动态计算 */
const heightPx = ref<number>(LINE_HEIGHT_PX + VERTICAL_PADDING_PX);

// ============================================================================
// expose：供父组件填入 draft（模板卡片点击、WelcomeInput 等）
// ============================================================================

function setDraft(content: string): void {
  draft.value = content;
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

function setImages(images: ImageItem[]): void {
  pendingImages.value = images;
}

async function addFiles(files: File[]): Promise<void> {
  await processFiles(files);
}

defineExpose({ setDraft, setImages, addFiles });

// ============================================================================
// autosize
// ============================================================================

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
// 键盘：Enter 发送；Shift/Ctrl/Meta+Enter 换行
// ============================================================================

function onKeydown(e: KeyboardEvent): void {
  if (e.key !== "Enter") return;
  if (e.shiftKey || e.ctrlKey || e.metaKey) return;
  e.preventDefault();
  handleSend();
}

// ============================================================================
// 提交
// ============================================================================

const inputDisabled = computed<boolean>(
  () => props.disabled || props.streaming,
);

function handleSend(): void {
  if (inputDisabled.value) return;
  const v = draft.value.trim();
  const hasImages = pendingImages.value.length > 0;
  if (!v && !hasImages) return;

  if (hasImages) {
    const blocks: ContentBlock[] = [];
    if (v) blocks.push({ type: "text", text: v });
    for (const img of pendingImages.value) {
      blocks.push({ type: "image", data: img.data, media_type: img.media_type });
    }
    emit("send", v, blocks);
  } else {
    emit("send", v);
  }

  draft.value = "";
  pendingImages.value = [];
  void nextTick(autosize);
  chatStore.setAppliedTemplate(null);
}

function onSendClick(): void {
  handleSend();
}

function onStopClick(): void {
  emit("stop");
}

const sendDisabled = computed<boolean>(
  () =>
    inputDisabled.value ||
    (draft.value.trim().length === 0 && pendingImages.value.length === 0),
);

/** textarea 占位文案（不同状态下不同） */
const placeholderText = computed<string>(() => {
  if (props.streaming) return "生成中…";
  return "输入消息，Enter 发送，Shift+Enter 换行";
});

</script>

<template>
  <div
    :class="[
      'ip-chat-input',
      {
        'ip-chat-input--disabled': disabled,
        'ip-chat-input--streaming': streaming,
      },
    ]"
  >
    <!-- =================================================================== -->
    <!-- textarea 容器（占位）                                                -->
    <!-- =================================================================== -->
    <div class="textarea-wrapper">
      <textarea
        ref="textareaRef"
        v-model="draft"
        class="textarea"
        :placeholder="placeholderText"
        :disabled="inputDisabled"
        rows="1"
        :style="{ height: `${heightPx}px` }"
        :maxlength="20000"
        @keydown="onKeydown"
        @paste="onPaste"
      />
    </div>

    <!-- =================================================================== -->
    <!-- toolbar: [Paperclip] [chip][chip]…[+N] [spacer] [发送/停止]            -->
    <!-- =================================================================== -->
    <div class="toolbar">
      <button
        type="button"
        class="btn-attach"
        :disabled="imagePickerDisabled"
        :title="
          imagePickerDisabled
            ? !supportsVision
              ? '当前 Agent 不支持图片输入'
              : '生成中，无法添加'
            : '添加图片（最多 20 张，单张 5MB）'
        "
        aria-label="添加附件"
        @click="triggerFilePicker"
      >
        <Paperclip :size="16" aria-hidden="true" />
      </button>

      <!-- 图片 chip 行（最多 4 个 + "+N"） -->
      <div
        v-if="pendingImages.length > 0"
        class="image-chips"
        :aria-label="`已添加 ${pendingImages.length} 张图片`"
      >
        <div
          v-for="(img, idx) in visibleImages"
          :key="img.preview"
          class="image-chip"
        >
          <img class="image-chip__thumb" :src="img.preview" alt="" />
          <span class="image-chip__name" :title="img.fileName ?? `图片 ${idx + 1}`">
            {{ img.fileName ?? `图片 ${idx + 1}` }}
          </span>
          <button
            type="button"
            class="image-chip__remove"
            :disabled="streaming"
            :aria-label="`移除第 ${idx + 1} 张图片`"
            @click="removeImage(idx)"
          >
            <X :size="10" aria-hidden="true" />
          </button>
        </div>
        <!-- +N 溢出 chip：<=3 用 title，>3 用 Popconfirm -->
        <IpPopconfirm
          v-if="overflowCount > 3"
          v-model="overflowPopoverOpen"
          :title="`还有 ${overflowCount} 张图片`"
          :description="overflowNamesText"
          confirm-text="关闭"
          cancel-text="关闭"
          placement="top"
          width="240"
        >
          <template #trigger>
            <button
              type="button"
              class="image-chip image-chip--more"
              :aria-label="`还有 ${overflowCount} 张图片，点击查看`"
            >
              +{{ overflowCount }}
            </button>
          </template>
        </IpPopconfirm>
        <button
          v-else-if="overflowCount > 0"
          type="button"
          class="image-chip image-chip--more"
          :title="`还有 ${overflowCount} 张图片：${overflowNamesText}`"
          :aria-label="`还有 ${overflowCount} 张图片`"
        >
          +{{ overflowCount }}
        </button>
      </div>

      <div class="toolbar-spacer" />

      <button
        v-if="streaming"
        type="button"
        class="btn btn-stop"
        title="停止生成"
        aria-label="停止生成"
        @click="onStopClick"
      >
        <Square :size="14" aria-hidden="true" />
        <span class="btn-label">停止</span>
      </button>
      <button
        v-else
        type="button"
        class="btn btn-send"
        :disabled="sendDisabled"
        title="发送（Enter）"
        aria-label="发送消息"
        @click="onSendClick"
      >
        <SendHorizontal :size="16" aria-hidden="true" />
        <span class="btn-label">发送</span>
      </button>
    </div>

    <!-- 隐藏的 file input（驱动附件按钮 click） -->
    <input
      ref="fileInputRef"
      type="file"
      :accept="ACCEPT_ATTR"
      multiple
      class="file-input-hidden"
      aria-hidden="true"
      tabindex="-1"
      @change="onFileChange"
    />
  </div>
</template>

<style scoped>
/* ============================================================================
 * ChatInput 视觉实现
 * ========================================================================== */

.ip-chat-input {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
  border-top: 1px solid var(--ip-color-border-default);
  background-color: var(--ip-color-bg-secondary);
  padding: var(--ip-spacing-3) var(--ip-spacing-4);
  flex-shrink: 0;
  position: relative;
}

/* ============ Textarea 容器（relative 给角标定位） ============ */
.textarea-wrapper {
  position: relative;
  width: 100%;
}

.textarea {
  display: block;
  width: 100%;
  resize: none;
  border: 1px solid var(--ip-input-border);
  border-radius: var(--ip-input-radius);
  padding: var(--ip-input-py-md) var(--ip-input-px-md);
  font-family: inherit;
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-text-body-lh);
  color: var(--ip-color-text-primary);
  background: var(--ip-input-bg);
  outline: none;
  box-sizing: border-box;
  overflow-y: auto;
  transition:
    border-color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out),
    background-color var(--ip-duration-base) var(--ip-ease-out);
}

.textarea::placeholder {
  color: var(--ip-color-text-tertiary);
}

.textarea:hover:not(:disabled) {
  border-color: var(--ip-color-border-strong);
}

.textarea:focus {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

.textarea:disabled {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  cursor: not-allowed;
}

/* ============ Toolbar ============ */
.toolbar {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
  min-height: 32px;
}

.toolbar-spacer {
  flex: 1;
}

/* ============ Paperclip 附件按钮 ============ */
.btn-attach {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: var(--ip-btn-h-toolbar);
  height: var(--ip-btn-h-toolbar);
  padding: 0;
  color: var(--ip-color-text-tertiary);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  flex-shrink: 0;
  transition: var(--ip-transition-colors);
}

.btn-attach:hover:not(:disabled) {
  color: var(--ip-color-text-secondary);
  background: var(--ip-color-bg-tertiary);
}

.btn-attach:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn-attach:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* ============ 图片 chip 行（28px 高） ============ */
.image-chips {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--ip-spacing-1_5);
  flex: 1;
  overflow: hidden;
}

.image-chip {
  display: inline-flex;
  align-items: center;
  gap: var(--ip-spacing-1);
  height: 28px;
  padding: 0 4px 0 8px;
  max-width: 140px;
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  flex-shrink: 0;
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out);
}

.image-chip:hover {
  border-color: var(--ip-color-border-strong);
}

.image-chip__thumb {
  width: 20px;
  height: 20px;
  border-radius: 3px;
  object-fit: cover;
  flex-shrink: 0;
}

.image-chip__name {
  font-size: var(--ip-text-body-sm-size);
  color: var(--ip-color-text-secondary);
  max-width: 80px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.image-chip__remove {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  color: var(--ip-color-text-tertiary);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  flex-shrink: 0;
  transition: color var(--ip-duration-fast) var(--ip-ease-out);
}

.image-chip__remove:hover {
  color: var(--ip-danger-base);
}

.image-chip--more {
  padding: 0 8px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  cursor: pointer;
  justify-content: center;
  gap: 0;
}

.image-chip--more:hover {
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-primary);
}

/* ============ 发送 / 停止按钮 ============ */
.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  appearance: none;
  height: var(--ip-btn-h-toolbar);
  padding: 0 14px;
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  border-radius: var(--ip-btn-radius);
  border: 1px solid transparent;
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn:active:not(:disabled) {
  transform: translateY(0.5px);
}

.btn-label {
  line-height: 1;
}

.btn-send {
  background: var(--ip-primary-600);
  color: var(--ip-color-text-on-primary);
  border-color: var(--ip-primary-600);
}

.btn-send:hover:not(:disabled) {
  background: var(--ip-primary-700);
  border-color: var(--ip-primary-700);
}

.btn-send:disabled {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-disabled);
  border-color: var(--ip-color-bg-tertiary);
  cursor: not-allowed;
}

.btn-stop {
  background: var(--ip-danger-base);
  color: var(--ip-color-text-on-danger);
  border-color: var(--ip-danger-base);
}

.btn-stop:hover {
  background: var(--ip-danger-hover);
  border-color: var(--ip-danger-hover);
}

.btn-stop:focus-visible {
  box-shadow: var(--ip-shadow-focus-danger);
}

.btn-stop:active {
  background: var(--ip-danger-active);
  border-color: var(--ip-danger-active);
}

.ip-chat-input--streaming .textarea {
  border-color: var(--ip-color-border-default);
}

/* ============ 流式 / 禁用 状态 ============ */
.ip-chat-input--disabled {
  opacity: 0.6;
  pointer-events: none;
}

/* ============ 隐藏的 file input ============ */
.file-input-hidden {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0, 0, 0, 0);
  white-space: nowrap;
  border: 0;
  pointer-events: none;
}
</style>

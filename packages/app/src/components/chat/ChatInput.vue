<script setup lang="ts">
// 聊天输入框
//
// 职责：
//   - 多行 textarea，自动增高（最多 6 行高度）
//   - Enter 发送；Shift+Enter / Ctrl+Enter / Meta+Enter 换行
//   - 右下发送按钮：默认显示 SendHorizontal 图标；流式中切换为 Square + 红色高亮
//   - 模板芯片行（P2-4）：横向滚动，点击 chip → 选中并应用模板
//   - @ 自动补全（P2-4）：输入 @ 触发模板补全 popover
//   - 全部样式走 --ip-input-* Design Token，焦点态使用 --ip-shadow-focus 发光环
//   - 暗色模式：颜色由 token 自动覆盖
//
// props:
//   - disabled:   是否禁用整个输入（无会话时由外层控制）
//   - streaming:  是否正在流式生成
//
// emits:
//   - send(content: string)  点击发送 / Enter 提交时
//   - stop()                点击停止按钮时

import { computed, nextTick, ref, watch, useTemplateRef } from "vue";
import { SendHorizontal, Square, Wrench, Library } from "lucide-vue-next";
import { useChatStore } from "../../stores/chat";
import { useTemplatesStore } from "../../stores/templates";
import TemplatePicker from "../template/TemplatePicker.vue";
import ImagePicker, { type ImageItem } from "./ImagePicker.vue";
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
const templatesStore = useTemplatesStore();

// ============================================================================
// 文本草稿
// ============================================================================

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

/** textarea DOM 引用 */
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
// 模板选择状态
// ============================================================================

/** 当前选中的模板 ID（与 chip 行同步） */
const selectedTemplateId = ref<string | null>(null);

/** TemplatePicker DOM 引用 */
const pickerRef = useTemplateRef<InstanceType<typeof TemplatePicker> | null>("pickerRef");

/**
 * @ 自动补全：检测 textarea 内容末尾的 @xxx 模式。
 * 仅在末尾出现「@xxx」且光标在末尾时触发，简化实现（不做 caret 定位）。
 */
const atQuery = computed<{ match: string; index: number } | null>(() => {
  const text = draft.value;
  if (!text) return null;
  // 匹配末尾的 @xxx（不含空白）
  const m = text.match(/(?:^|\s)@([^\s@]*)$/);
  if (!m || m.index === undefined) return null;
  // 跳过前导空白的位置
  return { match: m[1] ?? "", index: m.index + (m[0]!.length - m[1]!.length - 1) };
});

/** 是否处于 @ 自动补全态 */
const atActive = computed<boolean>(() => atQuery.value !== null);

watch(atActive, (active) => {
  if (active && atQuery.value) {
    // 估算位置：textarea 右下角
    void nextTick(() => {
      const el = textareaRef.value;
      if (!el) return;
      const rect = el.getBoundingClientRect();
      pickerRef.value?.openAutocomplete(
        { x: rect.left + 16, y: rect.top - 8 },
        atQuery.value!.match,
      );
    });
  } else {
    pickerRef.value?.closeAutocomplete();
  }
});

watch(atQuery, (q) => {
  if (q && atActive.value) {
    pickerRef.value?.openAutocomplete(
      // 位置以 textarea 左下角为锚
      computeAutocompleteAnchor(),
      q.match,
    );
  }
});

function computeAutocompleteAnchor(): { x: number; y: number } {
  const el = textareaRef.value;
  if (!el) return { x: 0, y: 0 };
  const rect = el.getBoundingClientRect();
  return { x: rect.left + 16, y: rect.top - 8 };
}

/** 监听 @ 键盘：上/下/Enter/Tab/Esc 由 picker 处理 */
function onKeydown(e: KeyboardEvent): void {
  // @ 补全态下优先让 picker 处理
  if (atActive.value && pickerRef.value) {
    const handled = pickerRef.value.onAutocompleteKey(e);
    if (handled) {
      e.preventDefault();
      // Enter 触发应用后，picker 关闭，textarea 文本需要去掉 @xxx
      if (e.key === "Enter" || e.key === "Tab") {
        stripAtQueryFromDraft();
      }
      return;
    }
  }
  if (e.key !== "Enter") return;
  // Shift+Enter / Ctrl+Enter / Cmd+Enter 走默认换行
  if (e.shiftKey || e.ctrlKey || e.metaKey) return;
  // 纯 Enter 触发发送
  e.preventDefault();
  handleSend();
}

/** 去掉 draft 末尾的 @xxx 段 */
function stripAtQueryFromDraft(): void {
  if (!atQuery.value) return;
  const text = draft.value;
  const start = atQuery.value.index;
  draft.value = text.slice(0, start);
  void nextTick(autosize);
}

// ============================================================================
// 模板应用回调
// ============================================================================

/**
 * TemplatePicker 的 @apply 事件：
 * - 把模板信息存到 chat store
 * - draft 同步展示提示：「已应用模板：XXX（变量已填）」
 *   实际发送时由 sendMessage 携带 templateId + values 给后端
 */
function onTemplateApply(payload: { templateId: string; values: Record<string, string> }): void {
  chatStore.setAppliedTemplate(payload);
  // 同步 chip 选中态
  selectedTemplateId.value = payload.templateId;
}

function onTemplateSelectedChange(id: string | null): void {
  selectedTemplateId.value = id;
  if (id === null) {
    chatStore.setAppliedTemplate(null);
  }
}

// ============================================================================
// expose：供父组件填入 draft（模板卡片点击）
// ============================================================================

/**
 * 外部设置 textarea 的 draft 内容（模板卡片点击时调用）。
 * 自动触发 autosize + 聚焦。
 */
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

defineExpose({ setDraft });

// ============================================================================
// autosize
// ============================================================================

/**
 * 根据 draft 内容计算 textarea 的高度（向上增高，封顶为 maxHeightPx）。
 */
function autosize(): void {
  const el = textareaRef.value;
  if (!el) return;
  el.style.height = `${LINE_HEIGHT_PX + VERTICAL_PADDING_PX}px`;
  const sh = el.scrollHeight;
  const h = Math.min(sh, maxHeightPx);
  el.style.height = `${h}px`;
  heightPx.value = h;
}

/** 监听 draft 变化触发 autosize；用户编辑时清空已应用模板 */
watch(draft, () => {
  void nextTick(autosize);
  // 用户编辑消息 → 清空已应用模板（避免发送时携带与文本不匹配的模板）
  if (chatStore.appliedTemplate) {
    chatStore.setAppliedTemplate(null);
  }
});

// ============================================================================
// 实际是否禁用输入（流中或外层禁用时） */
const inputDisabled = computed<boolean>(() => props.disabled || props.streaming);

/** 提交（Enter） */
function handleSend(): void {
  if (inputDisabled.value) return;
  const v = draft.value.trim();
  const hasImages = pendingImages.value.length > 0;
  // 必须提供文本或图片二者之一
  if (!v && !hasImages) return;

  // P2-2 多模态：若有图片则构造 content_blocks，否则只发文本
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

  // 重置
  draft.value = "";
  pendingImages.value = [];
  void nextTick(autosize);
  // 清空 chip 选中 + 应用模板
  selectedTemplateId.value = null;
  chatStore.setAppliedTemplate(null);
}

/** 工具栏发送按钮点击 */
function onSendClick(): void {
  handleSend();
}

/** 工具栏停止按钮点击 */
function onStopClick(): void {
  emit("stop");
}

/** 发送按钮是否禁用（无文本且无图片、或外层禁用） */
const sendDisabled = computed<boolean>(
  () =>
    inputDisabled.value ||
    (draft.value.trim().length === 0 && pendingImages.value.length === 0),
);

/** 已应用模板的展示文本（chip 旁的小提示） */
const appliedHint = computed<string | null>(() => {
  const tpl = chatStore.appliedTemplate;
  if (!tpl) return null;
  const meta = templatesStore.byId(tpl.templateId);
  if (!meta) return null;
  const filled = Object.keys(tpl.values).filter(
    (k) => (tpl.values[k] ?? "").length > 0,
  );
  return filled.length > 0 ? `${meta.name}（${filled.length} 个变量已填）` : meta.name;
});
</script>

<template>
  <div
    :class="['chat-input', { 'chat-input-disabled': disabled, 'chat-input-streaming': streaming }]"
  >
    <!-- 模板芯片行 -->
    <div class="picker-area">
      <TemplatePicker
        ref="pickerRef"
        :selected-id="selectedTemplateId"
        @update:selected-id="onTemplateSelectedChange"
        @apply="onTemplateApply"
      />
    </div>

    <!-- 已应用模板的提示 -->
    <div v-if="appliedHint && !streaming" class="applied-hint">
      ✨ 已应用：{{ appliedHint }}
    </div>

    <!-- P2-2 多模态：图片选择器（附件按钮 + 缩略图列表） -->
    <ImagePicker
      :images="pendingImages"
      :disabled="streaming"
      @update:images="onImagesChange"
    />

    <textarea
      ref="textareaRef"
      v-model="draft"
      class="textarea"
      :placeholder="streaming ? '生成中...' : '输入消息，回车发送，Shift+Enter 换行（输入 @ 触发模板）'"
      :disabled="inputDisabled"
      rows="1"
      :style="{ height: `${heightPx}px` }"
      :maxlength="20000"
      @keydown="onKeydown"
    />
    <div class="toolbar">
      <div class="toolbar-left">
        <!-- P2-1: 工具开关按钮 -->
        <button
          class="btn-tool-toggle"
          :class="{ 'btn-tool-toggle-active': chatStore.toolsEnabled }"
          type="button"
          :title="chatStore.toolsEnabled ? '已开启工具调用（LLM 可读写本地文件）' : '开启工具调用（LLM 可读写本地文件）'"
          :aria-label="chatStore.toolsEnabled ? '关闭工具调用' : '开启工具调用'"
          :aria-pressed="chatStore.toolsEnabled"
          @click="chatStore.toolsEnabled = !chatStore.toolsEnabled"
        >
          <Wrench :size="14" aria-hidden="true" />
          <span class="tool-toggle-label">工具</span>
        </button>
        <!-- Phase 1 disabled: 模板库按钮占位 -->
        <button
          class="btn-tool-toggle"
          type="button"
          disabled
          title="Phase 1.1 可用"
          aria-label="模板库（即将推出）"
        >
          <Library :size="14" aria-hidden="true" />
          <span class="tool-toggle-label">模板库</span>
        </button>
        <span class="hint-text">
          <template v-if="streaming">生成中 · 可随时停止</template>
          <template v-else>Enter 发送 · Shift+Enter 换行 · @ 选模板</template>
        </span>
      </div>
      <div class="toolbar-right">
        <button
          v-if="streaming"
          class="btn btn-stop"
          type="button"
          title="停止生成"
          aria-label="停止生成"
          @click="onStopClick"
        >
          <Square :size="14" aria-hidden="true" />
          <span class="btn-label">停止</span>
        </button>
        <button
          v-else
          class="btn btn-send"
          type="button"
          :disabled="sendDisabled"
          title="发送（Enter）"
          aria-label="发送消息"
          @click="onSendClick"
        >
          <SendHorizontal :size="16" aria-hidden="true" />
          <span class="btn-label">发送</span>
        </button>
      </div>
    </div>
  </div>
</template>

<style scoped>
.chat-input {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--ip-color-border-default);
  background-color: var(--ip-color-bg-secondary);
  padding: 12px 16px;
  flex-shrink: 0;
  position: relative;
}

.picker-area {
  width: 100%;
  margin-bottom: var(--ip-spacing-2);
}

.applied-hint {
  display: inline-flex;
  align-items: center;
  margin-bottom: var(--ip-spacing-2);
  padding: 4px 10px;
  font-size: var(--ip-text-caption-size);
  color: var(--ip-primary-700, #1d4ed8);
  background: var(--ip-primary-100, #dbeafe);
  border-radius: var(--ip-radius-sm);
  align-self: flex-start;
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

.toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-top: 10px;
  min-height: 32px;
}

.toolbar-left {
  display: flex;
  align-items: center;
  gap: 8px;
}

.hint-text {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
}

.toolbar-right {
  display: flex;
  align-items: center;
  gap: 8px;
}

.btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  appearance: none;
  height: var(--ip-btn-h-sm);
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

.chat-input-streaming .textarea {
  border-color: var(--ip-color-border-default);
}

/* P2-1: 工具开关按钮 */
.btn-tool-toggle {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  appearance: none;
  height: 28px;
  padding: 0 8px;
  font-size: var(--ip-text-caption-size);
  font-weight: var(--ip-font-weight-medium);
  font-family: inherit;
  border-radius: var(--ip-radius-sm);
  border: 1px solid var(--ip-color-border-default);
  background: transparent;
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition: var(--ip-transition-colors);
}

.btn-tool-toggle:hover {
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-secondary);
}

.btn-tool-toggle-active {
  border-color: var(--ip-primary-600);
  background: var(--ip-primary-100, #dbeafe);
  color: var(--ip-primary-700, #1d4ed8);
}

.btn-tool-toggle-active:hover {
  border-color: var(--ip-primary-600);
  color: var(--ip-primary-700, #1d4ed8);
}

/* 停用状态的按钮（模板库占位） */
.btn-tool-toggle:disabled {
  opacity: 0.45;
  cursor: not-allowed;
}

.btn-tool-toggle:disabled:hover {
  border-color: var(--ip-color-border-default);
  color: var(--ip-color-text-tertiary);
}

.tool-toggle-label {
  line-height: 1;
}
</style>

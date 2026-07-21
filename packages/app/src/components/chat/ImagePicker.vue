<script setup lang="ts">
// 图片选择器组件（P2-2 多模态输入）
//
// 职责：
//   - 附件按钮（lucide ImagePlus）→ 打开系统文件选择器（accept 限定 png/jpeg/gif/webp）
//   - 选择后读取为 data URL，显示缩略图
//   - 每张图带 X 删除按钮
//   - 限制 20 张 + 单张 5MB 前端预校验
//   - 输出 `{ data: base64, media_type, preview: data URL }[]`
//
// props:
//   - images: 当前已选择图片列表（受控）
//   - disabled: 是否禁用整个选择器（流式中）
//
// emits:
//   - update:images(images) — 列表更新时触发

import { computed, useTemplateRef } from "vue";
import { ImagePlus, X } from "lucide-vue-next";
import {
  useImageFiles,
  ACCEPT_ATTR,
  MAX_COUNT,
  type ImageItem,
} from "../../composables/useImageFiles";
import { useToast } from "../../composables/useToast";

// re-export 保持向后兼容（其他组件可能从此处 import type）
export type { ImageItem };

const props = defineProps<{
  images: ImageItem[];
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:images": [images: ImageItem[]];
}>();

const toast = useToast();

// ============================================================================
// 常量（从 composable 导入）
// ============================================================================

/** accept 属性（从 composable 导入） */
const ACCEPT = ACCEPT_ATTR;

const fileInputRef = useTemplateRef<HTMLInputElement | null>("fileInputRef");

// ============================================================================
// 文件处理（使用 composable）
// ============================================================================

const { processFiles } = useImageFiles(
  () => props.images,
  (images) => emit("update:images", images),
);

// ============================================================================
// 派生
// ============================================================================

/** 是否还可继续添加（未达上限） */
const canAddMore = computed<boolean>(
  () => props.images.length < MAX_COUNT && !props.disabled,
);

/** 提示文案中的剩余数 */
const remainingText = computed<string>(() => `${props.images.length} / ${MAX_COUNT}`);

// ============================================================================
// 行为
// ============================================================================

/** 触发原生文件选择器 */
function pickFiles(): void {
  if (!canAddMore.value) {
    toast.warning(`最多 ${MAX_COUNT} 张图片`);
    return;
  }
  fileInputRef.value?.click();
}

/** 文件 input change 回调：读 File → 转 data URL → 切出 base64 + media_type */
function onFileChange(e: Event): void {
  const target = e.target as HTMLInputElement | null;
  if (!target) return;
  const files = target.files;
  if (!files || files.length === 0) return;

  // 复制为数组以便后续 await 中也能遍历
  const fileList = Array.from(files);
  // 重置 input.value，允许重复选择同一文件
  target.value = "";

  void processFiles(fileList);
}

/** 删除指定下标的图片 */
function removeAt(idx: number): void {
  if (props.disabled) return;
  if (idx < 0 || idx >= props.images.length) return;
  const next = [...props.images];
  next.splice(idx, 1);
  emit("update:images", next);
}
</script>

<template>
  <div :class="['image-picker', { 'image-picker-disabled': disabled }]">
    <!-- 触发按钮 -->
    <button
      type="button"
      class="btn-attach"
      :disabled="!canAddMore"
      :title="canAddMore ? '添加图片（最多 20 张，单张 5MB）' : `已达上限 ${MAX_COUNT} 张`"
      :aria-label="canAddMore ? '添加图片' : '图片已达上限'"
      @click="pickFiles"
    >
      <ImagePlus :size="14" aria-hidden="true" />
      <span class="btn-label">图片</span>
      <span class="counter">{{ remainingText }}</span>
    </button>

    <!-- 隐藏的原生 file input -->
    <input
      ref="fileInputRef"
      type="file"
      :accept="ACCEPT"
      multiple
      class="file-input-hidden"
      aria-hidden="true"
      tabindex="-1"
      @change="onFileChange"
    />

    <!-- 缩略图列表 -->
    <ul v-if="images.length > 0" class="thumb-list" :aria-label="`已添加 ${images.length} 张图片`">
      <li v-for="(img, idx) in images" :key="idx" class="thumb-item">
        <img class="thumb-img" :src="img.preview" :alt="`附件 ${idx + 1}`" />
        <button
          type="button"
          class="thumb-remove"
          :disabled="disabled"
          :title="`移除第 ${idx + 1} 张`"
          :aria-label="`移除第 ${idx + 1} 张图片`"
          @click="removeAt(idx)"
        >
          <X :size="12" aria-hidden="true" />
        </button>
      </li>
    </ul>
  </div>
</template>

<style scoped>
.image-picker {
  display: flex;
  flex-direction: column;
  gap: var(--ip-spacing-2);
}

.image-picker-disabled {
  opacity: 0.6;
  pointer-events: none;
}

/* ===== 触发按钮（与 ChatInput 内 Wrench 按钮风格保持一致） ===== */
.btn-attach {
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

.btn-attach:hover:not(:disabled) {
  border-color: var(--ip-color-border-strong);
  color: var(--ip-color-text-secondary);
}

.btn-attach:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.btn-attach:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.btn-label {
  line-height: 1;
}

.counter {
  font-size: var(--ip-text-caption-size, 11px);
  color: var(--ip-color-text-quaternary, #999);
  font-variant-numeric: tabular-nums;
  margin-left: 2px;
}

/* ===== 隐藏原生 input ===== */
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

/* ===== 缩略图列表 ===== */
.thumb-list {
  display: flex;
  flex-wrap: wrap;
  gap: var(--ip-spacing-2);
  list-style: none;
  margin: 0;
  padding: 0;
}

.thumb-item {
  position: relative;
  width: 64px;
  height: 64px;
  border-radius: var(--ip-radius-sm);
  overflow: hidden;
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-tertiary);
  flex-shrink: 0;
}

.thumb-img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.thumb-remove {
  position: absolute;
  top: 2px;
  right: 2px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border-radius: 50%;
  border: none;
  background: rgba(0, 0, 0, 0.6);
  color: #fff;
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast, 150ms) var(--ip-ease-out),
    transform var(--ip-duration-fast, 150ms) var(--ip-ease-out);
}

.thumb-remove:hover:not(:disabled) {
  background: var(--ip-danger-base);
}

.thumb-remove:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus, 0 0 0 2px rgba(59, 130, 246, 0.4));
}

.thumb-remove:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
</style>

<script setup lang="ts">
// 图片选择器组件（P2-2 多模态输入）
//
// 职责：
//   - 附件按钮（lucide ImagePlus）→ 打开系统文件选择器（accept 限定 png/jpeg/gif/webp）
//   - 选择后读取为 data URL，显示缩略图
//   - 每张图带 ✕ 删除按钮
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
import { useToast } from "../../composables/useToast";

/** 组件对外的图片条目 */
export interface ImageItem {
  /** 裸 base64 字符串（不含 `data:image/...;base64,` 前缀） */
  data: string;
  /** MIME 类型，例如 `image/png` */
  media_type: string;
  /** 完整的 data URL（含前缀，仅用于 `<img src>` 预览） */
  preview: string;
}

const props = defineProps<{
  images: ImageItem[];
  disabled?: boolean;
}>();

const emit = defineEmits<{
  "update:images": [images: ImageItem[]];
}>();

const toast = useToast();

// ============================================================================
// 常量
// ============================================================================

/** 接受的文件 MIME（与 Rust 侧白名单 + input accept 属性对齐） */
const ACCEPT = "image/png,image/jpeg,image/gif,image/webp";

/** 单张最大字节数（5MB） */
const MAX_FILE_SIZE = 5 * 1024 * 1024;

/** 总数上限（与 Rust 侧校验一致） */
const MAX_COUNT = 20;

const fileInputRef = useTemplateRef<HTMLInputElement | null>("fileInputRef");

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

async function processFiles(files: File[]): Promise<void> {
  const slots = MAX_COUNT - props.images.length;
  if (slots <= 0) {
    toast.warning(`最多 ${MAX_COUNT} 张图片`);
    return;
  }
  const toProcess = files.slice(0, slots);
  if (files.length > slots) {
    toast.warning(`超过上限，已截取前 ${slots} 张`);
  }

  const additions: ImageItem[] = [];
  for (const f of toProcess) {
    // 大小预校验
    if (f.size > MAX_FILE_SIZE) {
      toast.error(`图片「${f.name || "未命名"}」超过 5MB，已跳过`);
      continue;
    }
    // MIME 预校验（accept 已经过滤一遍，但拖拽/paste 兜底）
    if (!f.type || !ACCEPT.split(",").includes(f.type)) {
      toast.error(`不支持的图片格式：${f.type || "未知"}，仅支持 png/jpeg/gif/webp`);
      continue;
    }

    // FileReader.readAsDataURL → "data:image/png;base64,xxxx"
    try {
      const dataUrl = await readAsDataURL(f);
      const { base64, mediaType } = splitDataUrl(dataUrl, f.type);
      additions.push({ data: base64, media_type: mediaType, preview: dataUrl });
    } catch (err) {
      const msg = err instanceof Error ? err.message : String(err);
      toast.error(`读取图片失败：${msg}`);
    }
  }

  if (additions.length > 0) {
    emit("update:images", [...props.images, ...additions]);
  }
}

function readAsDataURL(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const r = reader.result;
      if (typeof r === "string") resolve(r);
      else reject(new Error("FileReader 返回非字符串"));
    };
    reader.onerror = () => reject(new Error(reader.error?.message ?? "读取失败"));
    reader.readAsDataURL(file);
  });
}

/** 拆出 base64 主段与 media type。FileReader 输出固定是 `data:<type>;base64,<data>` */
function splitDataUrl(
  dataUrl: string,
  fallbackType: string,
): { base64: string; mediaType: string } {
  const m = /^data:([^;,]+);base64,(.*)$/.exec(dataUrl);
  if (!m || !m[1] || !m[2]) {
    // 极小概率：格式异常 → 返回原 data 段
    return { base64: dataUrl, mediaType: fallbackType };
  }
  return { base64: m[2], mediaType: m[1] };
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

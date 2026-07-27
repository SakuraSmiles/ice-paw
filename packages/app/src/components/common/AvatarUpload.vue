<script setup lang="ts">
// 头像上传组件（REQ-AGENT-002）
//
// 职责：
//   - 提供点击 / 拖拽 / 粘贴 上传入口
//   - 上传的图片在浏览器端读取为 base64 data URL（JPEG / PNG）
//   - 圆形预览（border-radius: 50%），保证视觉一致
//   - 把结果（data URL）通过 v-model / update:modelValue 抛出
//   - 提供「清除」按钮可一键移除
//
// 用法（典型用法见 AgentForm.vue）：
//   <AvatarUpload v-model="avatar" :size="80" />
//
// props:
//   - modelValue: 当前头像（base64 data URL），null/undefined 表示未上传
//   - size:      预览尺寸（正方形），默认 80
//   - maxBytes:  文件大小上限（字节），默认 2MB
//
// emits:
//   - update:modelValue [dataUrl | null]
//
// 实现要点：
//   - 直接使用 <input type="file"> + dragenter/dragover/drop 事件
//   - 通过 FileReader.readAsDataURL 转 base64（与后端的 schema TEXT 列直接对接）
//   - 不需要图像处理 / resize（base64 长度由前端 maxBytes 兜底）

import { ref } from "vue";
import { Camera, Trash2, Upload } from "lucide-vue-next";

const props = withDefaults(
  defineProps<{
    /** 当前头像：base64 data URL，null/undefined 表示未上传 */
    modelValue?: string | null;
    /** 预览尺寸（正方形），px 单位 */
    size?: number;
    /** 文件大小上限（字节），超过则拒绝 */
    maxBytes?: number;
  }>(),
  {
    modelValue: null,
    size: 80,
    maxBytes: 2 * 1024 * 1024, // 2MB
  },
);

const emit = defineEmits<{
  "update:modelValue": [value: string | null];
  /** 上传失败（type/size 校验不通过 / FileReader 错误）时触发 */
  error: [message: string];
}>();

// ============================================================================
// 内部状态
// ============================================================================

/** 真实 <input type="file"> 元素的引用（点击 / 拖拽都要复用） */
const fileInputRef = ref<HTMLInputElement | null>(null);

/** 拖拽态高亮 */
const isDragOver = ref<boolean>(false);

/** 当前加载中（FileReader 读取中） */
const reading = ref<boolean>(false);

// ============================================================================
// 派生
// ============================================================================

/** 是否有上传头像（用于控制「清除」按钮显隐） */
const hasAvatar = (): boolean =>
  typeof props.modelValue === "string" && props.modelValue.length > 0;

// ============================================================================
// 用户交互
// ============================================================================

/** 点击预览 → 触发文件选择 */
function triggerFileDialog(): void {
  fileInputRef.value?.click();
}

/** <input type="file"> change 事件 */
function onFileChange(e: Event): void {
  const target = e.target as HTMLInputElement;
  const file = target.files?.[0];
  if (!file) return;
  handleSelectedFile(file);
  // 清空 value，让用户能再次选择同一文件也触发 change
  target.value = "";
}

/** 拖拽进入 */
function onDragEnter(e: DragEvent): void {
  e.preventDefault();
  if (reading.value) return;
  isDragOver.value = true;
}

/** 拖拽离开 */
function onDragLeave(e: DragEvent): void {
  e.preventDefault();
  // 仅当真正离开 wrapper 时才取消高亮（避免子元素冒泡）
  if (e.currentTarget === e.target) {
    isDragOver.value = false;
  }
}

/** 拖拽放下 */
function onDragOver(e: DragEvent): void {
  e.preventDefault();
}

/** 拖拽放下 → 读取第一个文件 */
function onDrop(e: DragEvent): void {
  e.preventDefault();
  isDragOver.value = false;
  if (reading.value) return;
  const file = e.dataTransfer?.files?.[0];
  if (!file) return;
  handleSelectedFile(file);
}

/** 清除头像 */
function clearAvatar(): void {
  emit("update:modelValue", null);
  if (fileInputRef.value) {
    fileInputRef.value.value = "";
  }
}

// ============================================================================
// 文件处理
// ============================================================================

/**
 * 校验 + 读取选中的图片文件。
 * - 类型仅允许 jpg / png（image/jpeg, image/png）
 * - 大小上限 maxBytes
 * - 成功 → 通过 update:modelValue 抛出 base64 data URL
 * - 失败 → 通过 error 事件抛出本地化消息
 */
function handleSelectedFile(file: File): void {
  // 类型校验
  if (file.type !== "image/jpeg" && file.type !== "image/png") {
    emit("error", "仅支持 JPG / PNG 图片");
    return;
  }
  // 大小校验
  if (file.size > props.maxBytes) {
    const mb = Math.round(props.maxBytes / 1024 / 1024);
    emit("error", `图片大小超过 ${mb} MB`);
    return;
  }
  reading.value = true;
  const reader = new FileReader();
  reader.onload = () => {
    reading.value = false;
    if (typeof reader.result === "string") {
      emit("update:modelValue", reader.result);
    } else {
      emit("error", "读取图片失败");
    }
  };
  reader.onerror = () => {
    reading.value = false;
    emit("error", "读取图片失败");
  };
  reader.readAsDataURL(file);
}
</script>

<template>
  <div
    class="avatar-upload"
    :class="{ 'avatar-upload--has-image': hasAvatar(), 'avatar-upload--dragover': isDragOver }"
    :style="{ width: `${size}px`, height: `${size}px` }"
    @dragenter="onDragEnter"
    @dragleave="onDragLeave"
    @dragover="onDragOver"
    @drop="onDrop"
  >
    <!-- 隐藏的文件 input -->
    <input
      ref="fileInputRef"
      type="file"
      accept="image/jpeg,image/png"
      class="avatar-upload__input"
      aria-hidden="true"
      tabindex="-1"
      @change="onFileChange"
    />

    <!-- 上传后展示图片 -->
    <img
      v-if="hasAvatar()"
      :src="modelValue ?? ''"
      alt="头像预览"
      class="avatar-upload__img"
      @click="triggerFileDialog"
    />

    <!-- 未上传 → 占位 + 上传提示 -->
    <button
      v-else
      type="button"
      class="avatar-upload__placeholder"
      :disabled="reading"
      :aria-label="reading ? '读取中' : '上传头像'"
      @click="triggerFileDialog"
    >
      <Camera v-if="!reading" :size="20" aria-hidden="true" />
      <Upload v-else :size="20" aria-hidden="true" />
      <span class="avatar-upload__placeholder-text">
        {{ reading ? "读取中…" : "上传头像" }}
      </span>
    </button>

    <!-- 覆盖层（hover 显示「更换」徽章） -->
    <button
      v-if="hasAvatar()"
      type="button"
      class="avatar-upload__overlay"
      aria-label="更换头像"
      @click="triggerFileDialog"
    >
      <Camera :size="16" aria-hidden="true" />
    </button>

    <!-- 清除按钮（右上角，仅在有头像时显示） -->
    <button
      v-if="hasAvatar()"
      type="button"
      class="avatar-upload__clear"
      aria-label="清除头像"
      title="清除头像"
      @click="clearAvatar"
    >
      <Trash2 :size="14" aria-hidden="true" />
    </button>
  </div>
</template>

<style scoped>
.avatar-upload {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  border-radius: 50%;
  background: var(--ip-color-bg-secondary);
  border: 1px dashed var(--ip-color-border-default);
  overflow: hidden;
  cursor: pointer;
  transition:
    border-color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.avatar-upload:hover {
  border-color: var(--ip-color-border-strong);
}

.avatar-upload--dragover {
  border-color: var(--ip-color-border-focus);
  background: var(--ip-color-bg-tertiary);
}

.avatar-upload--has-image {
  border-style: solid;
}

.avatar-upload__input {
  position: absolute;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
}

.avatar-upload__img {
  width: 100%;
  height: 100%;
  border-radius: 50%;
  object-fit: cover;
  user-select: none;
}

.avatar-upload__placeholder {
  appearance: none;
  width: 100%;
  height: 100%;
  border: none;
  background: none;
  border-radius: 50%;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 4px;
  color: var(--ip-color-text-tertiary);
  font-family: inherit;
  transition:
    color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.avatar-upload__placeholder:hover {
  color: var(--ip-color-text-primary);
  background: var(--ip-color-bg-tertiary);
}

.avatar-upload__placeholder:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.avatar-upload__placeholder:disabled {
  cursor: wait;
  opacity: 0.7;
}

.avatar-upload__placeholder-text {
  font-size: 10px;
  font-weight: var(--ip-font-weight-medium, 500);
  line-height: 1.2;
  letter-spacing: 0.01em;
}

.avatar-upload__overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(0, 0, 0, 0.45);
  color: #fff;
  border: none;
  border-radius: 50%;
  cursor: pointer;
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
}

.avatar-upload:hover .avatar-upload__overlay,
.avatar-upload:focus-within .avatar-upload__overlay {
  opacity: 1;
}

.avatar-upload__overlay:focus-visible {
  outline: none;
  opacity: 1;
  box-shadow: var(--ip-shadow-focus);
}

.avatar-upload__clear {
  position: absolute;
  top: 2px;
  right: 2px;
  width: 22px;
  height: 22px;
  border-radius: 50%;
  border: 1px solid var(--ip-color-border-default);
  background: var(--ip-color-bg-elevated);
  color: var(--ip-color-text-secondary);
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  opacity: 0;
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    color var(--ip-duration-fast) var(--ip-ease-out),
    border-color var(--ip-duration-fast) var(--ip-ease-out);
}

.avatar-upload:hover .avatar-upload__clear,
.avatar-upload:focus-within .avatar-upload__clear {
  opacity: 1;
}

.avatar-upload__clear:hover {
  color: var(--ip-color-text-error, #dc2626);
  border-color: var(--ip-color-text-error, #dc2626);
}

.avatar-upload__clear:focus-visible {
  outline: none;
  opacity: 1;
  box-shadow: var(--ip-shadow-focus);
}
</style>

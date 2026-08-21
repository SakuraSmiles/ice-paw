<!--
  AvatarField — 头像上传统一组件（用户拍板 2026-08-21 简化形态）

  单一形态：头像框本体即交互（hover「更换」蒙层 + 右上角小 × 清空）。
  无独立编辑态/按钮排/体积文案——点击框 → 裁剪器（内含选图）→ 确认更新。
  三通道：点击 / 拖入图片 / Ctrl+V 粘贴 → 直达裁剪器。
  清空：hover 右上 × （danger 色 hover 提示；无确认弹窗——操作可逆）。

  Props: name: string（EntityAvatar 渐变兜底名）
         modelValue?: string | null（头像 dataURL，v-model）
  Emits: update:modelValue（dataURL | null——null = 清空）
-->
<script setup lang="ts">
import { ref } from "vue";
import EntityAvatar from "./EntityAvatar.vue";
import AvatarCropper from "./AvatarCropper.vue";

defineProps<{
  name: string;
  modelValue?: string | null;
  /** 主题色 hex（项目场景：EntityAvatar 渐变兜底档优先用） */
  accent?: string | null;
}>();

const emit = defineEmits<{ "update:modelValue": [v: string | null] }>();

// ---- 裁剪器 ----
const cropperOpen = ref(false);
// 待裁剪原图（点击进入时若已有 modelValue 图则作为 source 直接定位）
const cropSource = ref<string | null>(null);

function openCropper(source?: string | null) {
  cropSource.value = source ?? null;
  cropperOpen.value = true;
}

function onCropConfirm(data: string) {
  cropperOpen.value = false;
  emit("update:modelValue", data);
}

function clearAvatar() {
  emit("update:modelValue", null);
}

// ---- 拖拽 / 粘贴通道 ----
const dragOver = ref(false);
const boxRef = ref<HTMLElement | null>(null);

function onDragOver(e: DragEvent) {
  if (!e.dataTransfer?.types?.includes("Files")) return;
  e.preventDefault();
  dragOver.value = true;
}
function onDragLeave(e: DragEvent) {
  if (e.currentTarget === e.target) dragOver.value = false;
}
function onDrop(e: DragEvent) {
  const f = e.dataTransfer?.files?.[0];
  if (!f) return;
  e.preventDefault();
  dragOver.value = false;
  void ingestFile(f);
}
async function onPaste(e: ClipboardEvent) {
  const f = e.clipboardData?.files?.[0];
  if (!f) return;
  e.preventDefault();
  await ingestFile(f);
}

/** 文件 → dataURL 原图 → 直达裁剪器（2MB 校验前置，超限 inline 报错）。 */
const errMsg = ref("");
async function ingestFile(f: File) {
  errMsg.value = "";
  if (!f.type.startsWith("image/")) {
    errMsg.value = "仅支持图片文件";
    return;
  }
  // 大小校验（与 utils/avatar 的 AVATAR_MAX_SRC_BYTES 同源值，提前拦截免裁剪器内失败）
  if (f.size > 2 * 1024 * 1024) {
    errMsg.value = `图片过大（${(f.size / 1024 / 1024).toFixed(1)}MB），请选择 2MB 以内的图片`;
    return;
  }
  const url = URL.createObjectURL(f);
  openCropper(url);
}
</script>

<template>
  <div class="avatar-field">
    <!-- 头像框（无图：虚线占位；有图：头像 + hover 蒙层） -->
    <div
      ref="boxRef"
      class="af-box"
      :class="{ 'is-empty': !modelValue, 'drag-over': dragOver }"
      :title="modelValue ? '点击调整头像' : '点击上传头像'"
      tabindex="0"
      role="button"
      :aria-label="modelValue ? '调整头像' : '上传头像'"
      @click="openCropper(modelValue)"
      @keydown.enter.prevent="openCropper(modelValue)"
      @dragover="onDragOver"
      @dragleave="onDragLeave"
      @drop="onDrop"
      @paste="onPaste"
    >
      <!-- 始终渲染：无图自动走名字渐变兜底（与展示位一致的降级链） -->
      <EntityAvatar :name="name" :image="modelValue" :accent="accent" class="af-avatar" />

      <!-- hover 蒙层（有图时） -->
      <span v-if="modelValue" class="af-mask">更换</span>
      <!-- 清空 ×（有图时右上角；stopPropagation 防触发打开） -->
      <button
        v-if="modelValue"
        type="button"
        class="af-clear"
        title="移除头像"
        aria-label="移除头像"
        @click.stop="clearAvatar"
      >
        <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="3" stroke-linecap="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg>
      </button>
    </div>

    <span class="af-hint" :class="{ 'is-error': !!errMsg }">{{ errMsg || (modelValue ? "点击调整 · 支持拖入 / 粘贴" : "点击上传 · 支持拖入 / 粘贴") }}</span>

    <!-- 裁剪器弹层 -->
    <AvatarCropper
      v-if="cropperOpen"
      :source="cropSource"
      @confirm="onCropConfirm"
      @cancel="cropperOpen = false"
    />
  </div>
</template>

<style scoped>
.avatar-field {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-3);
}

/* 头像框：64px（lg EntityAvatar 的放大档——表单语境） */
.af-box {
  position: relative;
  width: 64px;
  height: 64px;
  border-radius: var(--ip-radius-lg);
  overflow: hidden;
  flex-shrink: 0;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  /* 无图：虚线占位 */
  border: 1.5px dashed var(--ip-color-border-default);
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-tertiary);
  transition: border-color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.af-box:not(.is-empty) {
  border-color: transparent; /* 有图：隐形边框占位（不画虚线） */
  background: none;
}
.af-box:focus-visible {
  outline: 2px solid var(--ip-primary-500);
  outline-offset: 2px;
}
.af-box.is-empty:hover, .af-box.drag-over {
  border-color: var(--ip-primary-400);
  background: var(--ip-color-primary-soft-bg);
}
.af-avatar {
  width: 100%;
  height: 100%;
  border-radius: var(--ip-radius-lg);
}
.af-placeholder-icon { color: inherit; }

/* hover 蒙层（有图）：与查看态同一语言 */
.af-mask {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(20, 30, 45, 0.55);
  color: #fff;
  font-size: var(--ip-text-micro-size);
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
  border-radius: var(--ip-radius-lg);
}
.af-box:hover .af-mask { opacity: 1; }

/* 清空 ×：右上角小圆钮（图片预览 remove 同款视觉基因） */
.af-clear {
  position: absolute;
  top: 3px;
  right: 3px;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  border: none;
  background: rgba(0, 0, 0, 0.5);
  color: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out), background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.af-box:hover .af-clear { opacity: 1; }
.af-clear:hover { background: var(--ip-danger-base); }

.af-hint {
  font-size: var(--ip-text-micro-size);
  color: var(--ip-color-text-tertiary);
}
/* 错误态：danger 色提示（inline 纯文本规范） */
.af-hint.is-error { color: var(--ip-danger-base); }
</style>

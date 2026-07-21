<script setup lang="ts">
/**
 * IpAvatarUpload — IcePaw 带裁剪的头像上传
 *
 * 点击触发文件选择 → 弹出裁剪 Dialog
 * Dialog：左侧画布 + 右侧实时预览 + 操作栏
 * 确认后输出 base64 data URL
 *
 * 裁剪方案：cropperjs@2（基于 Web Components，30KB gzip）
 * Tauri 环境检测：__TAURI__ 时使用 Tauri 文件对话框
 *
 * a11y：Dialog role="dialog", 焦点 trap, Esc 关闭
 */
import { ref, computed, nextTick, onUnmounted } from 'vue'
import { Camera, ZoomIn, ZoomOut, RotateCw, RotateCcw, RefreshCw, X, Check, Upload } from 'lucide-vue-next'
import Cropper from 'cropperjs'
import type { CropperCanvas, CropperImage, CropperSelection } from 'cropperjs'
import type { AvatarUploadProps, AvatarUploadEmits, AvatarUploadErrorInfo } from './types'

const props = withDefaults(defineProps<AvatarUploadProps>(), {
  modelValue: null,
  maxSize: 2 * 1024 * 1024,
  borderRadius: 'circle',
  disabled: false,
})

const emit = defineEmits<AvatarUploadEmits>()

/* ----- 状态 ----- */
const dialogVisible = ref(false)
const imageSrc = ref<string | null>(null)
const imageElRef = ref<HTMLImageElement | null>(null)
const canvasElRef = ref<HTMLElement | null>(null)
const cropperInstance = ref<Cropper | null>(null)
const fileInputRef = ref<HTMLInputElement | null>(null)
const dialogRef = ref<HTMLElement | null>(null)
const previouslyFocused = ref<HTMLElement | null>(null)
const isProcessing = ref(false)

/* ----- 焦点管理 ----- */
const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]),' +
  ' select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/* ----- Tauri 环境检测 ----- */
const isTauri = typeof window !== 'undefined' && '__TAURI__' in window

/* ----- 点击触发文件选择 ----- */
async function triggerFilePicker(): Promise<void> {
  if (props.disabled) return
  if (isTauri) {
    await pickFileTauri()
  } else {
    fileInputRef.value?.click()
  }
}

/* ----- Tauri 文件对话框（懒加载） ----- */
async function pickFileTauri(): Promise<void> {
  try {
    // 动态导入避免在非 Tauri 环境产生副作用
    // 使用字符串拼接 + @vite-ignore 避免 Rollup 解析未安装的包
    const moduleName = '@tauri-apps/plugin-dialog'
    const dialogModule: { open?: (opts: unknown) => Promise<string | null>; default?: { open?: (opts: unknown) => Promise<string | null> } } = await import(/* @vite-ignore */ moduleName)
    const open = dialogModule.open ?? dialogModule.default?.open
    if (typeof open !== 'function') {
      emitError('load_failed', 'Tauri dialog 插件不可用')
      return
    }
    const selected = await open({
      multiple: false,
      filters: [{ name: '图片', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'] }],
    })
    if (!selected || typeof selected !== 'string') return
    // 把本地路径转换为 asset URL（Tauri 2.x）
    const tauri = (window as unknown as { __TAURI__?: { core?: { convertFileSrc?: (p: string) => string } } }).__TAURI__
    const url = tauri?.core?.convertFileSrc ? tauri.core.convertFileSrc(selected) : selected
    const response = await fetch(url)
    const blob = await response.blob()
    const file = new File([blob], selected.split(/[/\\]/).pop() ?? 'avatar', { type: blob.type })
    handleFile(file)
  } catch (err) {
    emitError('load_failed', `Tauri 文件读取失败：${(err as Error).message ?? String(err)}`)
  }
}

/* ----- 文件选择处理 ----- */
function onFileChange(ev: Event): void {
  const target = ev.target as HTMLInputElement
  const file = target.files?.[0]
  target.value = '' // 重置，允许选择同名文件

  if (!file) return
  handleFile(file)
}

/* ----- 处理文件 ----- */
function handleFile(file: File): void {
  /* MIME 校验 */
  if (!file.type.startsWith('image/')) {
    emitError('invalid_mime', `不支持的文件类型：${file.type || '未知'}`)
    return
  }

  /* 大小校验 */
  if (file.size > props.maxSize) {
    const mb = (props.maxSize / 1024 / 1024).toFixed(1)
    emitError('file_too_large', `文件超过 ${mb}MB`)
    return
  }

  /* 读取文件 */
  const reader = new FileReader()
  reader.onload = () => {
    imageSrc.value = reader.result as string
    dialogVisible.value = true
    previouslyFocused.value = document.activeElement as HTMLElement | null
    document.body.style.overflow = 'hidden'
    nextTick(() => initCropper())
  }
  reader.onerror = () => {
    emitError('load_failed', '文件读取失败')
  }
  reader.readAsDataURL(file)
}

/* ----- 初始化 Cropper (v2 Web Components) ----- */
async function initCropper(): Promise<void> {
  if (!imageElRef.value) return

  // 销毁旧实例
  destroyCropper()

  cropperInstance.value = new Cropper(imageElRef.value, {
    container: canvasElRef.value ?? undefined,
  })

  // 等图片加载完成后聚焦 Dialog（选择框在 cropper-image 加载完后自动出现）
  const cropperImage = cropperInstance.value.getCropperImage()
  if (cropperImage) {
    try {
      await cropperImage.$ready()
    } catch {
      /* ignore */
    }
  }
  nextTick(() => focusFirst())
}

/* ----- 销毁 Cropper ----- */
function destroyCropper(): void {
  if (cropperInstance.value) {
    cropperInstance.value.destroy()
    cropperInstance.value = null
  }
}

/* ----- 获取 Cropper 子组件的便捷访问 ----- */
function getImage(): CropperImage | null {
  return cropperInstance.value?.getCropperImage() ?? null
}

function getSelection(): CropperSelection | null {
  return cropperInstance.value?.getCropperSelection() ?? null
}

function getCanvas(): CropperCanvas | null {
  return cropperInstance.value?.getCropperCanvas() ?? null
}

/* ----- 裁剪操作 ----- */
function zoomIn(): void {
  getImage()?.$zoom(0.1)
}

function zoomOut(): void {
  getImage()?.$zoom(-0.1)
}

function rotateLeft(): void {
  getImage()?.$rotate(-90)
}

function rotateRight(): void {
  getImage()?.$rotate(90)
}

function resetCropper(): void {
  getImage()?.$resetTransform()
  // 重置选区位置
  getSelection()?.$reset()
}

/* ----- 确认裁剪 ----- */
async function confirmCrop(): Promise<void> {
  if (!cropperInstance.value || isProcessing.value) return
  const canvasEl = getCanvas()
  if (!canvasEl) return

  isProcessing.value = true
  try {
    // $toCanvas 返回裁剪后的 HTMLCanvasElement
    const croppedCanvas = await canvasEl.$toCanvas({
      width: 512,
      height: 512,
    })
    // 转换为 base64 data URL
    const dataUrl = croppedCanvas.toDataURL('image/png', 0.92)
    emit('update:modelValue', dataUrl)
    closeDialog()
  } catch (err) {
    emitError('load_failed', `裁剪失败：${(err as Error).message ?? String(err)}`)
  } finally {
    isProcessing.value = false
  }
}

/* ----- 关闭 Dialog ----- */
function closeDialog(): void {
  destroyCropper()
  dialogVisible.value = false
  imageSrc.value = null
  document.body.style.overflow = ''
  previouslyFocused.value?.focus?.()
}

/* ----- Esc 关闭 + Tab 焦点循环 ----- */
function onKeydown(ev: KeyboardEvent): void {
  if (ev.key === 'Escape') {
    ev.stopPropagation()
    closeDialog()
    return
  }
  // Tab 循环
  if (ev.key === 'Tab' && dialogRef.value) {
    const focusable = dialogRef.value.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)
    if (focusable.length === 0) {
      ev.preventDefault()
      return
    }
    const first = focusable[0]
    const last = focusable[focusable.length - 1]
    const active = document.activeElement as HTMLElement | null

    if (ev.shiftKey && (active === first || !dialogRef.value.contains(active))) {
      ev.preventDefault()
      last.focus()
    } else if (!ev.shiftKey && active === last) {
      ev.preventDefault()
      first.focus()
    }
  }
}

/* ----- 焦点管理 ----- */
function focusFirst(): void {
  const root = dialogRef.value
  if (!root) return
  const focusable = root.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)
  const first = focusable[0]
  if (first) {
    first.focus()
  } else {
    root.focus()
  }
}

function onOverlayClick(): void {
  closeDialog()
}

/* ----- 错误 emit ----- */
function emitError(code: AvatarUploadErrorInfo['code'], message: string): void {
  emit('upload-error', { code, message })
}

/* ----- 清除头像 ----- */
function clearAvatar(): void {
  if (props.disabled) return
  emit('update:modelValue', null)
}

/* ----- 清理 ----- */
onUnmounted(() => {
  destroyCropper()
  if (dialogVisible.value) {
    document.body.style.overflow = ''
  }
})

/* ----- 派生样式 ----- */
const previewRadiusClass = computed(() =>
  props.borderRadius === 'circle' ? 'ip-avatar-upload__preview-viewer--circle' : '',
)
</script>

<template>
  <div :class="['ip-avatar-upload', { 'ip-avatar-upload--disabled': disabled }]">
    <!-- 头像预览 / 触发区 -->
    <button
      type="button"
      class="ip-avatar-upload__trigger"
      :disabled="disabled"
      :aria-label="modelValue ? '更换头像' : '上传头像'"
      @click="triggerFilePicker"
    >
      <!-- 已有图片 -->
      <img
        v-if="modelValue"
        :src="modelValue"
        alt="头像"
        class="ip-avatar-upload__current-image"
      />
      <!-- 空态 -->
      <div v-else class="ip-avatar-upload__placeholder">
        <Upload :size="20" aria-hidden="true" />
      </div>

      <!-- hover 蒙层 -->
      <div class="ip-avatar-upload__trigger-overlay" aria-hidden="true">
        <Camera :size="18" />
      </div>
    </button>

    <!-- 已上传时的清除按钮 -->
    <button
      v-if="modelValue && !disabled"
      type="button"
      class="ip-avatar-upload__remove"
      aria-label="移除头像"
      @click="clearAvatar"
    >
      <X :size="10" />
    </button>

    <!-- 隐藏文件 input（非 Tauri 环境） -->
    <input
      v-if="!isTauri"
      ref="fileInputRef"
      type="file"
      accept="image/*"
      class="ip-avatar-upload__file-input"
      tabindex="-1"
      aria-hidden="true"
      @change="onFileChange"
    />

    <!-- 裁剪 Dialog -->
    <Teleport to="body">
      <Transition name="ip-avatar-upload-dialog">
        <div
          v-if="dialogVisible"
          class="ip-avatar-upload__overlay"
          @mousedown.self="onOverlayClick"
          @keydown="onKeydown"
        >
          <div
            ref="dialogRef"
            class="ip-avatar-upload__dialog"
            role="dialog"
            aria-modal="true"
            aria-label="裁剪头像"
            tabindex="-1"
          >
            <!-- Header -->
            <header class="ip-avatar-upload__header">
              <h2 class="ip-avatar-upload__title">裁剪头像</h2>
              <button
                type="button"
                class="ip-avatar-upload__close"
                aria-label="关闭"
                @click="closeDialog"
              >
                <X :size="16" />
              </button>
            </header>

            <!-- Body: 左侧画布 + 右侧预览 -->
            <section class="ip-avatar-upload__body">
              <div class="ip-avatar-upload__canvas">
                <div ref="canvasElRef" class="ip-avatar-upload__canvas-inner">
                  <img
                    v-if="imageSrc"
                    ref="imageElRef"
                    :src="imageSrc"
                    alt="待裁剪图片"
                    class="ip-avatar-upload__crop-image"
                  />
                </div>
              </div>
              <div class="ip-avatar-upload__preview">
                <div :class="['ip-avatar-upload__preview-viewer', previewRadiusClass]">
                  <cropper-viewer
                    v-if="imageSrc"
                    :src="imageSrc"
                    class="ip-avatar-upload__preview-viewer-inner"
                  />
                </div>
                <p class="ip-avatar-upload__preview-label">预览</p>
              </div>
            </section>

            <!-- 操作栏 -->
            <footer class="ip-avatar-upload__toolbar">
              <div class="ip-avatar-upload__toolbar-actions">
                <button type="button" class="ip-avatar-upload__tool-btn" title="缩小" @click="zoomOut">
                  <ZoomOut :size="16" />
                </button>
                <button type="button" class="ip-avatar-upload__tool-btn" title="放大" @click="zoomIn">
                  <ZoomIn :size="16" />
                </button>
                <button type="button" class="ip-avatar-upload__tool-btn" title="左旋" @click="rotateLeft">
                  <RotateCcw :size="16" />
                </button>
                <button type="button" class="ip-avatar-upload__tool-btn" title="右旋" @click="rotateRight">
                  <RotateCw :size="16" />
                </button>
                <button type="button" class="ip-avatar-upload__tool-btn" title="重置" @click="resetCropper">
                  <RefreshCw :size="16" />
                </button>
              </div>

              <div class="ip-avatar-upload__toolbar-footer">
                <button
                  type="button"
                  class="ip-avatar-upload__btn-cancel"
                  @click="closeDialog"
                >
                  取消
                </button>
                <button
                  type="button"
                  class="ip-avatar-upload__btn-confirm"
                  :disabled="isProcessing"
                  @click="confirmCrop"
                >
                  <Check :size="16" />
                  <span>{{ isProcessing ? '处理中...' : '确认' }}</span>
                </button>
              </div>
            </footer>
          </div>
        </div>
      </Transition>
    </Teleport>
  </div>
</template>

<style scoped>
/* ============================================================
 * IpAvatarUpload — 带裁剪的头像上传
 * ============================================================ */

/* ----- 隐藏 file input ----- */
.ip-avatar-upload__file-input {
  position: absolute;
  opacity: 0;
  pointer-events: none;
  width: 1px;
  height: 1px;
  overflow: hidden;
}

/* ----- 触发器区域 ----- */
.ip-avatar-upload {
  position: relative;
  display: inline-flex;
  flex-shrink: 0;
}

.ip-avatar-upload--disabled {
  opacity: 0.5;
  pointer-events: none;
}

.ip-avatar-upload__trigger {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 80px;
  height: 80px;
  overflow: hidden;
  border-radius: 50%;
  border: 2px dashed var(--ip-color-border-default);
  background: var(--ip-color-bg-tertiary);
  cursor: pointer;
  padding: 0;
  transition:
    border-color var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-avatar-upload__trigger:hover {
  border-color: var(--ip-color-border-strong);
  background: var(--ip-color-bg-tertiary);
}

.ip-avatar-upload__trigger:active {
  transform: scale(0.97);
}

.ip-avatar-upload__trigger:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* 已上传时 border 改为 solid */
.ip-avatar-upload__trigger:has(.ip-avatar-upload__current-image) {
  border-style: solid;
  border-color: var(--ip-color-border-default);
}

.ip-avatar-upload__current-image {
  width: 100%;
  height: 100%;
  object-fit: cover;
}

.ip-avatar-upload__placeholder {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--ip-color-icon-muted);
  gap: var(--ip-spacing-1);
}

/* hover 蒙层 */
.ip-avatar-upload__trigger-overlay {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-avatar-overlay-light);
  color: var(--ip-color-text-on-primary);
  opacity: 0;
  transition: opacity var(--ip-duration-fast) var(--ip-ease-out);
  pointer-events: none;
  border-radius: inherit;
}

.ip-avatar-upload__trigger:hover .ip-avatar-upload__trigger-overlay,
.ip-avatar-upload__trigger:focus-visible .ip-avatar-upload__trigger-overlay {
  opacity: 1;
}

/* ----- 移除按钮 ----- */
.ip-avatar-upload__remove {
  position: absolute;
  bottom: -4px;
  right: -4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 22px;
  height: 22px;
  padding: 0;
  color: var(--ip-white);
  background: var(--ip-avatar-remove-bg);
  border: 2px solid var(--ip-color-bg-secondary);
  border-radius: 50%;
  cursor: pointer;
  z-index: 1;
  transition:
    transform var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-avatar-upload__remove:hover {
  background: var(--ip-danger-hover);
  transform: scale(1.1);
}

.ip-avatar-upload__remove:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * Dialog
 * ============================================================ */

/* ----- Overlay ----- */
.ip-avatar-upload__overlay {
  position: fixed;
  inset: 0;
  background: var(--ip-color-bg-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: var(--ip-z-modal-overlay);
  padding: var(--ip-spacing-6);
}

/* ----- Dialog 面板 ----- */
.ip-avatar-upload__dialog {
  width: 680px;
  max-width: calc(100vw - 64px);
  max-height: calc(100vh - 64px);
  background: var(--ip-color-bg-elevated);
  border-radius: var(--ip-modal-radius);
  box-shadow: var(--ip-shadow-lg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  outline: none;
}

/* ----- Header ----- */
.ip-avatar-upload__header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--ip-spacing-5) var(--ip-spacing-6);
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}

.ip-avatar-upload__title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
  margin: 0;
}

.ip-avatar-upload__close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  color: var(--ip-color-icon-default);
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-md);
  cursor: pointer;
  flex-shrink: 0;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-avatar-upload__close:hover {
  background: var(--ip-color-bg-tertiary);
}

.ip-avatar-upload__close:active {
  transform: scale(0.92);
}

.ip-avatar-upload__close:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ----- Body: 画布 + 预览 ----- */
.ip-avatar-upload__body {
  display: flex;
  gap: var(--ip-spacing-6);
  padding: var(--ip-spacing-6);
  flex: 1;
  overflow: hidden;
  min-height: 0;
}

.ip-avatar-upload__canvas {
  flex: 1;
  min-width: 0;
  min-height: 280px;
  max-height: 400px;
  background: var(--ip-color-bg-tertiary);
  border-radius: var(--ip-radius-md);
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}

.ip-avatar-upload__canvas-inner {
  width: 100%;
  height: 100%;
  min-height: 280px;
}

/* cropperjs canvas 元素 */
.ip-avatar-upload__canvas-inner :deep(cropper-canvas) {
  width: 100%;
  height: 100%;
  display: block;
}

/* cropperjs 内部的 img */
.ip-avatar-upload__canvas-inner :deep(cropper-canvas img) {
  max-width: 100%;
}

/* 默认隐藏被 cropperjs 包裹的 <img>（因为它会被 cropperjs 移动到 cropper-canvas 内） */
.ip-avatar-upload__crop-image {
  display: none;
}

/* ----- 预览区 ----- */
.ip-avatar-upload__preview {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: var(--ip-spacing-2);
  flex-shrink: 0;
  width: 140px;
}

.ip-avatar-upload__preview-viewer {
  width: 120px;
  height: 120px;
  overflow: hidden;
  border-radius: var(--ip-radius-md);
  background: var(--ip-color-bg-tertiary);
  border: 1px solid var(--ip-color-border-default);
}

.ip-avatar-upload__preview-viewer--circle {
  border-radius: 50%;
}

.ip-avatar-upload__preview-viewer-inner {
  width: 100%;
  height: 100%;
  display: block;
}

.ip-avatar-upload__preview-viewer-inner :deep(cropper-canvas) {
  width: 100%;
  height: 100%;
}

.ip-avatar-upload__preview-label {
  font-size: var(--ip-text-caption-size);
  color: var(--ip-color-text-tertiary);
  margin: 0;
}

/* ----- 操作栏 ----- */
.ip-avatar-upload__toolbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: var(--ip-spacing-4) var(--ip-spacing-6);
  border-top: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
  gap: var(--ip-spacing-4);
}

.ip-avatar-upload__toolbar-actions {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-1);
}

.ip-avatar-upload__toolbar-footer {
  display: flex;
  align-items: center;
  gap: var(--ip-spacing-2);
}

/* 操作按钮 */
.ip-avatar-upload__tool-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 32px;
  height: 32px;
  padding: 0;
  color: var(--ip-color-icon-default);
  background: transparent;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-radius-sm);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    border-color     var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-avatar-upload__tool-btn:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.ip-avatar-upload__tool-btn:active {
  transform: scale(0.92);
}

.ip-avatar-upload__tool-btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* 取消按钮 */
.ip-avatar-upload__btn-cancel {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  height: var(--ip-btn-h-md);
  padding: 0 var(--ip-btn-px-md);
  font-size: var(--ip-btn-fs-md);
  font-family: inherit;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-secondary);
  background: transparent;
  border: 1px solid var(--ip-color-border-default);
  border-radius: var(--ip-btn-radius);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    border-color     var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-avatar-upload__btn-cancel:hover {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}

.ip-avatar-upload__btn-cancel:active {
  transform: scale(0.97);
}

.ip-avatar-upload__btn-cancel:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* 确认按钮 */
.ip-avatar-upload__btn-confirm {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--ip-spacing-1);
  height: var(--ip-btn-h-md);
  padding: 0 var(--ip-btn-px-md);
  font-size: var(--ip-btn-fs-md);
  font-family: inherit;
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-on-primary);
  background: var(--ip-primary-600);
  border: none;
  border-radius: var(--ip-btn-radius);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-avatar-upload__btn-confirm:hover:not(:disabled) {
  background: var(--ip-primary-700);
}

.ip-avatar-upload__btn-confirm:active:not(:disabled) {
  transform: scale(0.97);
}

.ip-avatar-upload__btn-confirm:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.ip-avatar-upload__btn-confirm:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * Transition
 * ============================================================ */
.ip-avatar-upload-dialog-enter-active .ip-avatar-upload__overlay {
  animation: ip-au-overlay-in var(--ip-duration-modal) var(--ip-ease-out) both;
}
.ip-avatar-upload-dialog-enter-active .ip-avatar-upload__dialog {
  animation: ip-au-dialog-in var(--ip-duration-modal) var(--ip-ease-emphasized) 50ms both;
}
.ip-avatar-upload-dialog-leave-active .ip-avatar-upload__overlay {
  animation: ip-au-overlay-out var(--ip-duration-modal-out) var(--ip-ease-in) both;
}
.ip-avatar-upload-dialog-leave-active .ip-avatar-upload__dialog {
  animation: ip-au-dialog-out var(--ip-duration-modal-out) var(--ip-ease-in) both;
}

@keyframes ip-au-overlay-in {
  from { opacity: 0; }
  to   { opacity: 1; }
}
@keyframes ip-au-overlay-out {
  from { opacity: 1; }
  to   { opacity: 0; }
}
@keyframes ip-au-dialog-in {
  from { opacity: 0; transform: scale(0.96) translateY(-8px); }
  to   { opacity: 1; transform: scale(1) translateY(0); }
}
@keyframes ip-au-dialog-out {
  from { opacity: 1; transform: scale(1) translateY(0); }
  to   { opacity: 0; transform: scale(0.96) translateY(-8px); }
}
</style>
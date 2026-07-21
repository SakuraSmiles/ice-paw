<script setup lang="ts">
/**
 * Avatar — IcePaw 头像 / 项目图标组件
 *
 * 规范：icepaw-p0-component-specs.md §一
 * 微交互：
 *  - uploadable hover：蒙层 fade-in 120ms ease-out
 *  - removable hover：✕ 按钮 fade-in + scale-in 150ms ease-out
 *  - active 按下：scale(0.97) 50ms ease-out
 *  - loading：蒙层覆盖 + spinner 旋转
 *  - disabled：opacity 0.5 + cursor not-allowed
 * a11y：role="img" + aria-label 推断；uploadable/removable 时键盘 Tab 可达，Enter/Space 触发
 */
import { computed, ref } from 'vue'
import { Camera, FolderOpen, X } from 'lucide-vue-next'
import type { AvatarEmits, AvatarProps, AvatarSize, AvatarUploadError } from './types'

const props = withDefaults(defineProps<AvatarProps>(), {
  size: 'md',
  shape: 'rounded',
  uploadable: false,
  accept: 'image/png,image/jpeg,image/gif,image/webp',
  maxSize: 2 * 1024 * 1024,
  removable: false,
  loading: false,
  disabled: false,
})

const emit = defineEmits<AvatarEmits>()

const hovered = ref(false)
const fileInputRef = ref<HTMLInputElement | null>(null)

/* ----- 尺寸映射（token → px）----- */
const SIZE_MAP: Record<AvatarSize, number> = {
  xs: 20,
  sm: 28,
  md: 36,
  lg: 48,
  xl: 64,
  xxl: 96,
}
const pixelSize = computed<number>(() => SIZE_MAP[props.size])
const innerSize = computed<number>(() => Math.max(10, Math.floor(pixelSize.value * 0.5)))
const textFontSize = computed<number>(() => Math.max(10, Math.floor(pixelSize.value * 0.4)))
const removeSize = computed<number>(() => Math.max(12, Math.floor(pixelSize.value * 0.45)))

/* ----- 派生样式 ----- */
const rootStyle = computed<Record<string, string>>(() => ({
  width: `${pixelSize.value}px`,
  height: `${pixelSize.value}px`,
}))

/* ----- source 派生 ----- */
const inferredAlt = computed<string>(() => {
  if (props.source.type === 'image') return props.source.alt ?? props.alt ?? ''
  if (props.source.type === 'icon') return props.alt ?? '图标头像'
  if (props.source.type === 'initials') return props.alt ?? `${props.source.text} 头像`
  return props.alt ?? '占位头像'
})

/** 是否可交互（disabled / loading 时不可点击，不可上传） */
const isClickable = computed<boolean>(() => !props.disabled && !props.loading)
const showOverlay = computed<boolean>(
  () => props.uploadable && !props.disabled && !props.loading && hovered.value,
)
const showRemove = computed<boolean>(
  () =>
    props.removable &&
    props.source.type === 'image' &&
    !props.disabled &&
    !props.loading &&
    hovered.value,
)

/* ----- 文件选择 ----- */
function triggerFilePicker(): void {
  if (!isClickable.value || !props.uploadable) return
  fileInputRef.value?.click()
}

function onFileChange(ev: Event): void {
  const target = ev.target as HTMLInputElement
  const file = target.files?.[0]
  target.value = '' // 重置，允许选择同名文件
  if (!file) {
    emit('upload-error', { code: 'no_file', message: '未选择文件' } satisfies AvatarUploadError)
    return
  }
  /* MIME 校验 */
  const accepts = props.accept.split(',').map((s) => s.trim())
  if (!accepts.includes(file.type)) {
    emit('upload-error', {
      code: 'invalid_mime',
      message: `不支持的文件类型：${file.type || '未知'}`,
    } satisfies AvatarUploadError)
    return
  }
  /* 大小校验 */
  if (file.size > props.maxSize) {
    const mb = (props.maxSize / 1024 / 1024).toFixed(1)
    emit('upload-error', {
      code: 'file_too_large',
      message: `文件超过 ${mb}MB`,
    } satisfies AvatarUploadError)
    return
  }
  emit('upload', file)
}

function onClick(ev: MouseEvent): void {
  if (!isClickable.value) {
    ev.preventDefault()
    return
  }
  if (props.uploadable) {
    triggerFilePicker()
  }
  emit('click', ev)
}

function onKeydown(ev: KeyboardEvent): void {
  if (!isClickable.value) return
  if ((ev.key === 'Enter' || ev.key === ' ') && props.uploadable) {
    ev.preventDefault()
    triggerFilePicker()
  }
}

function onRemove(ev: MouseEvent): void {
  ev.stopPropagation()
  emit('remove')
}

function onMouseEnter(): void {
  if (!isClickable.value) return
  hovered.value = true
  emit('hover', true)
}
function onMouseLeave(): void {
  hovered.value = false
  emit('hover', false)
}
</script>

<template>
  <div
    :class="[
      'ip-avatar',
      `ip-avatar--${size}`,
      `ip-avatar--${shape}`,
      {
        'ip-avatar--uploadable': uploadable,
        'ip-avatar--removable': removable && source.type === 'image',
        'ip-avatar--loading': loading,
        'ip-avatar--disabled': disabled,
        'ip-avatar--hovered': hovered,
      },
    ]"
    :style="rootStyle"
    :role="'img'"
    :aria-label="ariaLabel ?? inferredAlt"
    :aria-busy="loading || undefined"
    :aria-disabled="disabled || undefined"
    :tabindex="(uploadable || removable) && isClickable ? 0 : undefined"
    @click="onClick"
    @keydown="onKeydown"
    @mouseenter="onMouseEnter"
    @mouseleave="onMouseLeave"
  >
    <!-- image 模式 -->
    <img
      v-if="source.type === 'image'"
      class="ip-avatar__image"
      :src="source.src"
      :alt="source.alt ?? ''"
    >

    <!-- icon 模式 -->
    <component
      :is="source.icon"
      v-else-if="source.type === 'icon'"
      class="ip-avatar__icon"
      :size="innerSize"
      :color="source.color ?? 'var(--ip-color-icon-default)'"
      :stroke-width="2.25"
      aria-hidden="true"
    />

    <!-- initials 模式 -->
    <span
      v-else-if="source.type === 'initials'"
      class="ip-avatar__initials"
      :style="{
        fontSize: `${textFontSize}px`,
        backgroundColor: source.bgColor,
        color: source.fgColor ?? 'inherit',
      }"
    >{{ source.text }}</span>

    <!-- default 模式 -->
    <component
      :is="source.icon ?? source.fallbackIcon ?? FolderOpen"
      v-else
      class="ip-avatar__icon"
      :size="innerSize"
      color="var(--ip-color-icon-muted)"
      :stroke-width="2"
      aria-hidden="true"
    />

    <!-- uploadable 蒙层 -->
    <div
      v-if="uploadable && !loading"
      :class="['ip-avatar__overlay', { 'ip-avatar__overlay--visible': showOverlay }]"
      aria-hidden="true"
    >
      <Camera
        :size="innerSize"
        color="var(--ip-color-text-on-primary)"
        :stroke-width="2"
      />
    </div>

    <!-- loading spinner -->
    <div v-if="loading" class="ip-avatar__spinner" aria-hidden="true" />

    <!-- removable ✕ -->
    <button
      v-if="removable && source.type === 'image' && !loading"
      type="button"
      :class="['ip-avatar__remove', { 'ip-avatar__remove--visible': showRemove }]"
      :style="{ width: `${removeSize}px`, height: `${removeSize}px` }"
      aria-label="移除图片"
      :tabindex="showRemove ? 0 : -1"
      @click="onRemove"
    >
      <X
        :size="Math.max(10, Math.floor(removeSize * 0.7))"
        color="white"
        :stroke-width="2.5"
      />
    </button>

    <!-- 隐藏 file input -->
    <input
      v-if="uploadable"
      ref="fileInputRef"
      type="file"
      class="ip-avatar__file-input"
      :accept="accept"
      tabindex="-1"
      aria-hidden="true"
      @change="onFileChange"
    >
  </div>
</template>

<style scoped>
/* ============================================================
 * Avatar — 根节点（§1.4.3）
 * ============================================================ */
.ip-avatar {
  background: var(--ip-color-bg-tertiary);
  box-shadow: var(--ip-avatar-border-shadow);
  /* 默认裁切；removable 时由 .ip-avatar--removable 单独放开 */
  overflow: hidden;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  user-select: none;
  position: relative;
  border: none;
  cursor: default;
  color: var(--ip-color-text-primary);

  /* §1.6 多属性过渡 */
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-base) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out),
    opacity var(--ip-duration-base) var(--ip-ease-out);
}

/* 尺寸档（§1.4.1） */
.ip-avatar--xs  { border-radius: var(--ip-radius-md); }
.ip-avatar--sm  { border-radius: var(--ip-radius-md); }
.ip-avatar--md  { border-radius: var(--ip-radius-md); }
.ip-avatar--lg  { border-radius: var(--ip-radius-lg); }
.ip-avatar--xl  { border-radius: var(--ip-radius-lg); }
.ip-avatar--xxl { border-radius: var(--ip-radius-xl); }

/* 形状（§1.4.2） */
.ip-avatar--circle { border-radius: 50%; }
.ip-avatar--rounded.ip-avatar--xs,
.ip-avatar--rounded.ip-avatar--sm,
.ip-avatar--rounded.ip-avatar--md { border-radius: var(--ip-radius-md); }
.ip-avatar--rounded.ip-avatar--lg,
.ip-avatar--rounded.ip-avatar--xl  { border-radius: var(--ip-radius-lg); }
.ip-avatar--rounded.ip-avatar--xxl { border-radius: var(--ip-radius-xl); }

/* ============================================================
 * Source 内容渲染（§1.4.4）
 * ============================================================ */
.ip-avatar__image {
  width: 100%;
  height: 100%;
  object-fit: cover;
  display: block;
}

.ip-avatar__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.ip-avatar__initials {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 100%;
  height: 100%;
  font-weight: var(--ip-font-weight-bold);
  letter-spacing: var(--ip-letter-spacing-snug);
  line-height: 1;
}

/* ============================================================
 * Uploadable 蒙层（§1.4.5）
 * ============================================================ */
.ip-avatar--uploadable {
  cursor: pointer;
}

.ip-avatar__overlay {
  position: absolute;
  inset: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-avatar-overlay-light);
  opacity: 0;
  pointer-events: none;
  transition:
    opacity var(--ip-duration-fast) var(--ip-ease-out),
    background-color var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-avatar__overlay--visible {
  opacity: 1;
}
[data-theme='dark'] .ip-avatar__overlay,
.dark .ip-avatar__overlay {
  background: var(--ip-avatar-overlay-dark);
}

.ip-avatar:active:not(.ip-avatar--disabled):not(.ip-avatar--loading) .ip-avatar__overlay--visible {
  background: var(--ip-avatar-overlay-light-active);
}
[data-theme='dark'] .ip-avatar:active:not(.ip-avatar--disabled):not(.ip-avatar--loading) .ip-avatar__overlay--visible,
.dark .ip-avatar:active:not(.ip-avatar--disabled):not(.ip-avatar--loading) .ip-avatar__overlay--visible {
  background: var(--ip-avatar-overlay-dark-active);
}

/* P0-1 fix：removable 时 ✕ 按钮溢出在容器外（right:-4px / bottom:-4px），
   不能被 overflow:hidden 裁切。同时子元素 image 必须继承圆角，
   否则取消 overflow:hidden 后图片会变成方形。 */
.ip-avatar--removable {
  overflow: visible;
}
.ip-avatar--removable .ip-avatar__image {
  border-radius: inherit;
}

/* ============================================================
 * Removable ✕ 按钮（§1.4.6）
 * ============================================================ */
.ip-avatar__remove {
  position: absolute;
  right: -4px;
  bottom: -4px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: var(--ip-avatar-remove-bg);
  color: white;
  border: none;
  border-radius: 50%;
  cursor: pointer;
  padding: 0;
  opacity: 0;
  transform: scale(0.8);
  pointer-events: none;
  transition:
    opacity var(--ip-duration-base) var(--ip-ease-out),
    transform var(--ip-duration-base) var(--ip-ease-out),
    box-shadow var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-avatar__remove--visible {
  opacity: 1;
  transform: scale(1);
  pointer-events: auto;
}
.ip-avatar__remove:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * Loading 模式（§1.4.7）
 * ============================================================ */
.ip-avatar--loading {
  pointer-events: none;
}
.ip-avatar--loading .ip-avatar__image,
.ip-avatar--loading .ip-avatar__icon,
.ip-avatar--loading .ip-avatar__initials {
  opacity: 0.5;
}

.ip-avatar__spinner {
  position: absolute;
  top: 50%;
  left: 50%;
  width: 50%;
  height: 50%;
  min-width: 14px;
  min-height: 14px;
  transform: translate(-50%, -50%);
  border: 1.5px solid rgba(0, 0, 0, 0.20);
  border-top-color: var(--ip-color-icon-default);
  border-radius: 50%;
  background: rgba(255, 255, 255, 0.6);
  animation: ip-spin var(--ip-duration-spinner) linear infinite;
}
[data-theme='dark'] .ip-avatar__spinner,
.dark .ip-avatar__spinner {
  background: rgba(0, 0, 0, 0.6);
  border-color: rgba(255, 255, 255, 0.20);
  border-top-color: var(--ip-color-icon-default);
}

/* ============================================================
 * Disabled 模式（§1.4.8）
 * ============================================================ */
.ip-avatar--disabled {
  cursor: not-allowed;
  pointer-events: none;
}
.ip-avatar--disabled .ip-avatar__image,
.ip-avatar--disabled .ip-avatar__icon,
.ip-avatar--disabled .ip-avatar__initials {
  opacity: 0.5;
}

/* ============================================================
 * Active 按下（§1.6）
 * ============================================================ */
.ip-avatar:active:not(.ip-avatar--disabled):not(.ip-avatar--loading) {
  transform: scale(0.97);
  transition: transform var(--ip-duration-btn-press) var(--ip-ease-out);
}

/* ============================================================
 * Focus-visible（键盘可达时显示 3px ring）
 * ============================================================ */
.ip-avatar:focus { outline: none; }
.ip-avatar:focus-visible {
  outline: none;
  box-shadow: var(--ip-avatar-border-shadow), var(--ip-shadow-focus);
}

/* ============================================================
 * 隐藏 file input
 * ============================================================ */
.ip-avatar__file-input {
  position: absolute;
  width: 1px;
  height: 1px;
  opacity: 0;
  pointer-events: none;
  /* keep keyboard focusable via root tabindex=0; hidden input 仅用于 click 触发 */
}
</style>
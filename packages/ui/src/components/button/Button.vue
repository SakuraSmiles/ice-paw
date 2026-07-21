<script setup lang="ts">
/**
 * Button — IcePaw 按钮组件
 *
 * Props：variant / size / loading / disabled / block / iconOnly / type
 * Slots：default / icon-left / icon-right
 * a11y：aria-busy=true 当 loading；aria-disabled=true 当 disabled；focus-visible 焦点环
 *
 * 微交互（icepaw-micro-interactions.md §1）：
 *  - hover:    translateY(-1px) + box-shadow-sm
 *  - active:   translateY(0) scale(0.97)（v1.0 由 0.98 修正为 0.97）
 *  - focus:    box-shadow-focus（3px ring，保持背景色不变）
 *  - loading:  content fade-out 100ms + spinner 150ms (delay 50ms)，
 *              spinner 720ms linear 旋转
 *  - disabled: gray-200 bg + gray-400 text，pointer-events none
 */
import { computed, ref } from 'vue'
import type { ButtonEmits, ButtonProps } from './types'

const props = withDefaults(defineProps<ButtonProps>(), {
  variant: 'primary',
  size: 'md',
  loading: false,
  disabled: false,
  block: false,
  iconOnly: false,
  type: 'button',
})

const emit = defineEmits<ButtonEmits>()

/* loading=true 自动 disabled */
const isDisabled = computed<boolean>(() => props.disabled || props.loading)
const isLoading = computed<boolean>(() => props.loading)

function onClick(ev: MouseEvent): void {
  if (isDisabled.value) {
    ev.preventDefault()
    return
  }
  emit('click', ev)
}

/* W6: 暴露 focus / blur 方法，供 Popconfirm 等父组件直接调用（如打开时聚焦 confirm） */
const rootEl = ref<HTMLButtonElement | null>(null)
function focus(): void {
  rootEl.value?.focus()
}
function blur(): void {
  rootEl.value?.blur()
}
defineExpose({ focus, blur })
</script>

<template>
  <button
    ref="rootEl"
    :type="type"
    :class="[
      'ip-btn',
      `ip-btn--${variant}`,
      `ip-btn--${size}`,
      {
        'ip-btn--block': block,
        'ip-btn--icon-only': iconOnly,
        'ip-btn--loading': isLoading,
        'ip-btn--disabled': isDisabled,
      },
    ]"
    :disabled="isDisabled"
    :aria-busy="isLoading || undefined"
    :aria-disabled="isDisabled || undefined"
    @click="onClick"
  >
    <span v-if="isLoading" class="ip-btn__spinner" aria-hidden="true" />
    <span v-else-if="$slots['icon-left']" class="ip-btn__icon ip-btn__icon-left">
      <slot name="icon-left" />
    </span>
    <span v-if="!iconOnly" class="ip-btn__label">
      <slot />
    </span>
    <span v-if="$slots['icon-right'] && !isLoading" class="ip-btn__icon ip-btn__icon-right">
      <slot name="icon-right" />
    </span>
  </button>
</template>

<style scoped>
/* ============================================================
 * Button — 基础结构（§1.2）
 * ============================================================ */
.ip-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  position: relative;
  gap: var(--ip-btn-gap);
  border-radius: var(--ip-btn-radius);
  font-family: inherit;
  font-weight: var(--ip-font-weight-medium);
  line-height: 1;
  cursor: pointer;
  user-select: none;
  white-space: nowrap;
  text-decoration: none;
  border: 1px solid transparent;
  -webkit-tap-highlight-color: transparent;

  /* 4 属性分时长过渡（§1.2）：
     bg/bc/cl 150ms — 颜色需要时间感知
     shadow 150ms — 与背景同步
     transform 100ms — 即时物理感 */
  transition:
    background-color var(--ip-duration-base) var(--ip-ease-out),
    border-color     var(--ip-duration-base) var(--ip-ease-out),
    color            var(--ip-duration-base) var(--ip-ease-out),
    box-shadow       var(--ip-duration-base) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);

  /* 视觉占位（按尺寸覆盖） */
  min-height: var(--ip-btn-h-md);
  padding: var(--ip-btn-py-md) var(--ip-btn-px-md);
  font-size: var(--ip-btn-fs-md);
}

/* ============================================================
 * 尺寸变体（§1.8）
 * ============================================================ */
.ip-btn--sm {
  min-height: var(--ip-btn-h-sm);
  padding: var(--ip-btn-py-sm) var(--ip-btn-px-sm);
  font-size: var(--ip-btn-fs-sm);
}
.ip-btn--md {
  min-height: var(--ip-btn-h-md);
  padding: var(--ip-btn-py-md) var(--ip-btn-px-md);
  font-size: var(--ip-btn-fs-md);
}
.ip-btn--lg {
  min-height: var(--ip-btn-h-lg);
  padding: var(--ip-btn-py-lg) var(--ip-btn-px-lg);
  font-size: var(--ip-btn-fs-lg);
}

/* 仅图标（圆形 hit area） */
.ip-btn--icon-only {
  padding: 0;
  width: var(--ip-btn-h-md);
}
.ip-btn--icon-only.ip-btn--sm { width: var(--ip-btn-h-sm); }
.ip-btn--icon-only.ip-btn--lg { width: var(--ip-btn-h-lg); }

/* 块级 */
.ip-btn--block { width: 100%; }

/* ============================================================
 * Focus（§1.5）— 用 box-shadow 而非 outline（更柔和）
 * 鼠标点击 :focus 不显示，仅 Tab :focus-visible 显示
 * ============================================================ */
.ip-btn:focus { outline: none; }

.ip-btn:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

/* ============================================================
 * Active（§1.4）— translateY(0) scale(0.97)，按下归位
 * transition 缩到 50ms：即时反馈
 * ============================================================ */
.ip-btn:active:not(.ip-btn--disabled):not(.ip-btn--loading) {
  transform: translateY(0) scale(0.97);
  transition:
    background-color var(--ip-duration-btn-press) var(--ip-ease-out),
    color            var(--ip-duration-btn-press) var(--ip-ease-out),
    border-color     var(--ip-duration-btn-press) var(--ip-ease-out),
    box-shadow       var(--ip-duration-btn-press) var(--ip-ease-out),
    transform        var(--ip-duration-btn-press) var(--ip-ease-out);
}

/* ============================================================
 * Variants — 颜色与微交互差异（§1.9）
 * ============================================================ */

/* ---------- Primary ---------- */
.ip-btn--primary {
  background: var(--ip-primary-500);
  color: var(--ip-color-text-on-primary);
  border-color: transparent;
}
.ip-btn--primary:hover:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-primary-600);
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-sm);
}
.ip-btn--primary:active:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-primary-700);
  transform: translateY(0) scale(0.97);
  box-shadow: none;
}
.ip-btn--primary:focus-visible {
  background: var(--ip-primary-500);
  box-shadow: var(--ip-shadow-focus);
}

/* ---------- Secondary ---------- */
.ip-btn--secondary {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
  border-color: var(--ip-color-border-default);
}
.ip-btn--secondary:hover:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-gray-200);
  border-color: var(--ip-color-border-strong);
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-sm);
}
.ip-btn--secondary:active:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-gray-300);
  border-color: var(--ip-gray-500);
  transform: translateY(0) scale(0.97);
  box-shadow: none;
}
.ip-btn--secondary:focus-visible {
  border-color: var(--ip-color-border-focus);
  box-shadow: var(--ip-shadow-focus);
}

/* ---------- Ghost ---------- */
.ip-btn--ghost {
  background: transparent;
  color: var(--ip-color-text-body);
  border-color: transparent;
}
.ip-btn--ghost:hover:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-text-primary);
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-xs);
}
.ip-btn--ghost:active:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-gray-200);
  transform: translateY(0) scale(0.97);
  box-shadow: none;
}
.ip-btn--ghost:focus-visible {
  box-shadow: var(--ip-shadow-focus);
}

/* ---------- Danger ---------- */
.ip-btn--danger {
  background: var(--ip-danger-base);
  color: var(--ip-color-text-on-danger);
  border-color: transparent;
}
.ip-btn--danger:hover:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-danger-hover);
  transform: translateY(-1px);
  box-shadow: var(--ip-shadow-sm);
}
.ip-btn--danger:active:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-danger-active);
  transform: translateY(0) scale(0.97);
  box-shadow: none;
}
.ip-btn--danger:focus-visible {
  background: var(--ip-danger-base);
  box-shadow: var(--ip-shadow-focus-danger);
}

/* ============================================================
 * Disabled / Loading（§1.7）
 *   不使用 opacity（避免 wash-out 后颜色不对 / 对比度不够）
 *   pointer-events: none 防止 hover 触发半透明态
 * ============================================================ */
.ip-btn--disabled,
.ip-btn--loading {
  cursor: not-allowed;
  opacity: 1;
  pointer-events: none;
  transform: none;
  box-shadow: none;
}

.ip-btn--primary.ip-btn--disabled,
.ip-btn--primary.ip-btn--loading,
.ip-btn--danger.ip-btn--disabled,
.ip-btn--danger.ip-btn--loading {
  background: var(--ip-gray-200);
  color: var(--ip-gray-400);
  border-color: transparent;
}

.ip-btn--secondary.ip-btn--disabled,
.ip-btn--secondary.ip-btn--loading {
  background: var(--ip-gray-100);
  color: var(--ip-gray-400);
  border-color: var(--ip-gray-200);
}

.ip-btn--ghost.ip-btn--disabled,
.ip-btn--ghost.ip-btn--loading {
  color: var(--ip-gray-400);
  background: transparent;
  border-color: transparent;
}

/* ============================================================
 * Loading 内容切换（§1.6）
 *   - spinner 始终 absolute 居中，避免布局抖动
 *   - 按钮宽度不变（不收缩为 spinner 尺寸）
 *   - label / icon 在 loading 时隐藏
 * ============================================================ */
.ip-btn__label,
.ip-btn__icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  transition:
    opacity   var(--ip-duration-fast) var(--ip-ease-out),
    transform var(--ip-duration-fast) var(--ip-ease-out);
}
.ip-btn--loading .ip-btn__label,
.ip-btn--loading .ip-btn__icon {
  opacity: 0;
  transform: scale(0.95);
}

/* ============================================================
 * Spinner（§1.6）
 *   1.5px 描边 + 顶部不透明（Lucide 描边语言）
 *   720ms 旋转（v1.0 由 1000ms 修正为 720ms）
 *   fade-in 150ms delay 50ms（避开 content 退出）
 * ============================================================ */
.ip-btn__spinner {
  position: absolute;
  width: 16px;
  height: 16px;
  border: 1.5px solid rgba(255, 255, 255, 0.30);
  border-top-color: var(--ip-color-text-on-primary);
  border-radius: var(--ip-radius-full);
  opacity: 0;
  transform: scale(0.5);
  animation:
    ip-spin var(--ip-duration-spinner) linear infinite,
    ip-spinner-in var(--ip-duration-base) var(--ip-ease-emphasized) var(--ip-duration-immediate) forwards;
}

/* 次级/ghost/secondary 变体的 spinner 颜色需要适配 */
.ip-btn--secondary .ip-btn__spinner {
  border-color: var(--ip-focus-ring-light);
  border-top-color: var(--ip-color-text-body);
}
.ip-btn--ghost .ip-btn__spinner {
  border-color: var(--ip-focus-ring-light);
  border-top-color: var(--ip-color-text-body);
}

/* ============================================================
 * 暗色模式适配（§1.10）
 * ============================================================ */
[data-theme='dark'] .ip-btn--secondary {
  background: transparent;
  border-color: var(--ip-color-border-default);
  color: var(--ip-color-text-body);
}
[data-theme='dark'] .ip-btn--secondary:hover:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-color-bg-tertiary);
  border-color: var(--ip-color-border-strong);
}
[data-theme='dark'] .ip-btn--ghost:hover:not(.ip-btn--disabled):not(.ip-btn--loading) {
  background: var(--ip-color-bg-tertiary);
}
</style>
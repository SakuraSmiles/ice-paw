<script setup lang="ts">
/**
 * Modal — IcePaw 模态弹窗
 *
 * 规范：icepaw-design-system.md §2.5
 * 微交互（icepaw-micro-interactions.md §5）：
 *  - open:   overlay 250ms ease-out + content 250ms ease-emphasized（50ms 延迟）
 *  - close:  overlay 200ms ease-in + content 200ms ease-in（同步）
 *  - focus trap（基础）：打开时聚焦第一个可聚焦元素，关闭后回到原焦点
 *  - Esc 关闭、遮罩点击关闭（@click.self）
 *  - 滚动锁定：打开时 body.overflow=hidden，关闭后 200ms 恢复
 *  - 嵌套：z-index = 1300 + 10×level（最多 2 层）
 */
import { computed, nextTick, onUnmounted, ref, watch } from 'vue'
import type { ModalEmits, ModalProps } from './types'

const props = withDefaults(defineProps<ModalProps>(), {
  modelValue: false,
  size: 'md',
  title: '',
  closeOnOverlay: true,
  closeOnEsc: true,
  showClose: true,
  nestedLevel: 0,
})

const emit = defineEmits<ModalEmits>()

const FOCUSABLE_SELECTOR =
  'a[href], button:not([disabled]), input:not([disabled]):not([type="hidden"]),' +
  ' select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])'

/* ----- 状态 ----- */
const visible = ref<boolean>(props.modelValue)
const isClosing = ref<boolean>(false)
const dialogRef = ref<HTMLElement | null>(null)
const previouslyFocused = ref<HTMLElement | null>(null)
const originalOverflow = ref<string>('')

/* ----- 派生 ----- */
const width = computed<string>(() => {
  const variant = props.size === 'sm' ? 'sm' : props.size === 'lg' ? 'lg' : 'md'
  return `var(--ip-modal-w-${variant})`
})

const overlayZ = computed<number>(() => {
  if (props.zIndex !== undefined) return props.zIndex
  return (1300) + props.nestedLevel * 10
})

const contentZ = computed<number>(() => {
  if (props.zIndex !== undefined) return props.zIndex + 10
  return (1310) + props.nestedLevel * 10
})

/* ----- 监听 v-model ----- */
watch(
  () => props.modelValue,
  (val) => {
    if (val) {
      // 最多 2 层嵌套（v1.0.1 §4），超过 warn
      if (props.nestedLevel >= 2) {
        console.warn(
          '[IcePaw Modal] 嵌套超过 2 层，建议改用 Popover / Drawer。',
        )
      }
      visible.value = true
      isClosing.value = false
      previouslyFocused.value = document.activeElement as HTMLElement | null

      // 滚动锁定（§5.7）
      originalOverflow.value = document.body.style.overflow
      document.body.style.overflow = 'hidden'

      nextTick(() => {
        focusFirst()
        emit('open')
      })
      document.addEventListener('keydown', onKeydown)
    } else {
      close()
    }
  },
)

onUnmounted(() => {
  document.removeEventListener('keydown', onKeydown)
  // 清理：组件卸载时恢复 overflow
  document.body.style.overflow = originalOverflow.value || ''
})

/* ----- 关闭流程：先退场动画 200ms，再隐藏 ----- */
function close(): void {
  if (isClosing.value) return
  isClosing.value = true
  document.removeEventListener('keydown', onKeydown)
  setTimeout(() => {
    visible.value = false
    isClosing.value = false
    emit('update:modelValue', false)
    emit('close')
    // 退场动画结束后恢复原焦点 + body overflow（§5.7 延迟到 afterLeave）
    previouslyFocused.value?.focus?.()
    document.body.style.overflow = originalOverflow.value || ''
  }, 200) // 与 §5.3 close 200ms 一致
}

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
  if (props.closeOnOverlay) {
    close()
  }
}

function onCloseButton(): void {
  close()
}

function onKeydown(ev: KeyboardEvent): void {
  if (ev.key === 'Escape' && props.closeOnEsc) {
    ev.stopPropagation()
    close()
    return
  }

  /* Tab 循环焦点（§5.6 P1 选做，但实现更友好） */
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
</script>

<template>
  <Teleport to="body">
    <Transition name="ip-modal">
      <div
        v-if="visible"
        class="ip-modal-overlay"
        :style="{ zIndex: overlayZ }"
        @mousedown.self="onOverlayClick"
      >
        <div
          ref="dialogRef"
          :class="[
            'ip-modal',
            `ip-modal--${size}`,
            { 'ip-modal--closing': isClosing },
          ]"
          :style="{ width, zIndex: contentZ }"
          role="dialog"
          aria-modal="true"
          :aria-label="title || '对话框'"
          tabindex="-1"
        >
          <!-- Header -->
          <header v-if="title || showClose || $slots.header" class="ip-modal__header">
            <slot name="header">
              <h2 v-if="title" class="ip-modal__title">{{ title }}</h2>
            </slot>
            <button
              v-if="showClose"
              type="button"
              class="ip-modal__close"
              aria-label="关闭"
              @click="onCloseButton"
            >
              <svg
                width="16"
                height="16"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="2"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <path d="M18 6 6 18" />
                <path d="m6 6 12 12" />
              </svg>
            </button>
          </header>

          <!-- Body -->
          <section class="ip-modal__body">
            <slot />
          </section>

          <!-- Footer -->
          <footer v-if="$slots.footer" class="ip-modal__footer">
            <slot name="footer" />
          </footer>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<style scoped>
/* ============================================================
 * Modal Overlay
 * ============================================================ */
.ip-modal-overlay {
  position: fixed;
  inset: 0;
  background: var(--ip-color-bg-overlay);
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 32px;
  z-index: var(--ip-z-modal-overlay);
}

/* ============================================================
 * Modal
 * ============================================================ */
.ip-modal {
  background: var(--ip-color-bg-elevated);
  border-radius: var(--ip-modal-radius);
  box-shadow: var(--ip-shadow-lg);
  display: flex;
  flex-direction: column;
  max-width: calc(100vw - 64px);
  max-height: calc(100vh - 64px);
  overflow: hidden;

  width: var(--ip-modal-w-md);
  z-index: var(--ip-z-modal-content);
  outline: none;
}

/* 尺寸变体 */
.ip-modal--sm { width: var(--ip-modal-w-sm); }
.ip-modal--md { width: var(--ip-modal-w-md); }
.ip-modal--lg { width: var(--ip-modal-w-lg); }

/* ============================================================
 * Header
 * ============================================================ */
.ip-modal__header {
  padding: 20px 24px 16px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--ip-spacing-4);
  border-bottom: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}

.ip-modal__title {
  font-size: var(--ip-text-h3-size);
  font-weight: var(--ip-font-weight-semibold);
  line-height: var(--ip-line-height-relaxed);
  color: var(--ip-color-text-primary);
  margin: 0;
}

.ip-modal__close {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-md);
  color: var(--ip-color-icon-default);
  cursor: pointer;
  flex-shrink: 0;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    transform        var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-modal__close:hover {
  background: var(--ip-color-bg-tertiary);
}

.ip-modal__close:active {
  transform: scale(0.92);
}

.ip-modal__close:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.ip-modal__close svg {
  width: 16px;
  height: 16px;
}

/* ============================================================
 * Body
 * ============================================================ */
.ip-modal__body {
  padding: var(--ip-spacing-6);
  overflow-y: auto;
  flex: 1;
  color: var(--ip-color-text-body);
  font-size: var(--ip-text-body-size);
  line-height: var(--ip-line-height-loose);
}

/* ============================================================
 * Footer
 * ============================================================ */
.ip-modal__footer {
  padding: 16px 24px 20px;
  display: flex;
  justify-content: flex-end;
  gap: var(--ip-spacing-2);
  border-top: 1px solid var(--ip-color-border-default);
  flex-shrink: 0;
}

/* ============================================================
 * Transition（§5.2 / §5.3）
 *   Enter: overlay 250ms + content 250ms (delay 50ms, ease-emphasized)
 *   Leave: overlay 200ms + content 200ms（同步开始，ease-in）
 * ============================================================ */

/* Enter — overlay 立即开始 */
.ip-modal-enter-active .ip-modal-overlay {
  animation: ip-modal-overlay-in var(--ip-duration-modal) var(--ip-ease-out) both;
}

/* Enter — content 延迟 50ms，让 overlay 先渲染（§5.2） */
.ip-modal-enter-active .ip-modal {
  animation: ip-modal-content-in var(--ip-duration-modal) var(--ip-ease-emphasized) var(--ip-duration-immediate) both;
}

/* Leave — overlay 与 content 同步开始（§5.3 同步 overlay+content，200ms） */
.ip-modal-leave-active .ip-modal-overlay {
  animation: ip-modal-overlay-out var(--ip-duration-modal-out) var(--ip-ease-in) both;
}
.ip-modal-leave-active .ip-modal {
  animation: ip-modal-content-out var(--ip-duration-modal-out) var(--ip-ease-in) both;
}
</style>
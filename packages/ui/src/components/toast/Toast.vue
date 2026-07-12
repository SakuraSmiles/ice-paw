<script setup lang="ts">
/**
 * Toast — 单条 Toast 视图
 *
 * 规范：icepaw-design-system.md §2.6
 * 微交互（icepaw-micro-interactions.md §6）：
 *  - enter: translateX(20px) + opacity 0 → 1，200ms ease-out
 *  - exit:  fade + collapse（高度+margin collapse 200ms，延迟 50ms）
 *  - hover 暂停倒计时（通过 emit pause/resume 给 useToast 处理）
 *  - merge: 内容 crossfade 150ms（无 enter/exit）
 */
import { computed } from 'vue'
import type { ToastInstance } from './types'

interface Props {
  toast: ToastInstance
  /** 是否启用倒计时（true 时悬停暂停） */
  pauseOnHover?: boolean
}

const props = withDefaults(defineProps<Props>(), {
  pauseOnHover: true,
})

const emit = defineEmits<{
  (e: 'close'): void
  (e: 'pause'): void
  (e: 'resume'): void
}>()

const typeClass = computed<string>(() => `ip-toast--${props.toast.type}`)

const icon = computed<string>(() => {
  switch (props.toast.type) {
    case 'success': return 'check-circle'
    case 'error':   return 'x-circle'
    case 'warning': return 'alert-triangle'
    case 'info':    return 'info'
    default:        return 'info'
  }
})

function onPause(): void {
  if (!props.pauseOnHover) return
  emit('pause')
}
function onResume(): void {
  if (!props.pauseOnHover) return
  emit('resume')
}
</script>

<template>
  <div
    :class="['ip-toast', typeClass, { 'ip-toast--merging': toast.isMerging }]"
    role="alert"
    aria-live="polite"
    @mouseenter="onPause"
    @mouseleave="onResume"
  >
    <!-- 图标（按类型配色） -->
    <span class="ip-toast__icon" aria-hidden="true">
      <svg v-if="icon === 'check-circle'" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M22 11.08V12a10 10 0 1 1-5.93-9.14" />
        <polyline points="22 4 12 14.01 9 11.01" />
      </svg>
      <svg v-else-if="icon === 'x-circle'" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="15" y1="9" x2="9" y2="15" />
        <line x1="9" y1="9" x2="15" y2="15" />
      </svg>
      <svg v-else-if="icon === 'alert-triangle'" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3Z" />
        <line x1="12" y1="9" x2="12" y2="13" />
        <line x1="12" y1="17" x2="12.01" y2="17" />
      </svg>
      <svg v-else width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <circle cx="12" cy="12" r="10" />
        <line x1="12" y1="16" x2="12" y2="12" />
        <line x1="12" y1="8" x2="12.01" y2="8" />
      </svg>
    </span>

    <!-- 主体（合并时 crossfade） -->
    <div class="ip-toast__body">
      <div v-if="toast.title" class="ip-toast__title">{{ toast.title }}</div>
      <div v-if="toast.message" class="ip-toast__message">{{ toast.message }}</div>
    </div>

    <!-- 关闭按钮 -->
    <button
      v-if="toast.closable"
      type="button"
      class="ip-toast__close"
      aria-label="关闭"
      @click="emit('close')"
    >
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
        <path d="M18 6 6 18" />
        <path d="m6 6 12 12" />
      </svg>
    </button>
  </div>
</template>

<style scoped>
/* ============================================================
 * Toast 单条规格（§6.2）
 * ============================================================ */
.ip-toast {
  display: flex;
  align-items: flex-start;
  gap: var(--ip-spacing-3);
  min-width: var(--ip-toast-min-w);
  max-width: var(--ip-toast-max-w);
  padding: 12px 16px;
  background: var(--ip-color-bg-elevated);
  border-radius: var(--ip-radius-lg);
  box-shadow: var(--ip-shadow-lg);
  border-left: 3px solid;
  pointer-events: auto;

  font-size: var(--ip-text-body-sm-size);
  line-height: var(--ip-line-height-loose3);
  color: var(--ip-color-text-body);
}

/* 类型 — 边框颜色与图标颜色 */
.ip-toast--success { border-left-color: var(--ip-success-base); }
.ip-toast--warning { border-left-color: var(--ip-warning-base); }
.ip-toast--error   { border-left-color: var(--ip-danger-base); }
.ip-toast--info    { border-left-color: var(--ip-info-base); }

.ip-toast--success .ip-toast__icon { color: var(--ip-success-base); }
.ip-toast--warning .ip-toast__icon { color: var(--ip-warning-base); }
.ip-toast--error   .ip-toast__icon { color: var(--ip-danger-base); }
.ip-toast--info    .ip-toast__icon { color: var(--ip-info-base); }

/* Icon */
.ip-toast__icon {
  width: 20px;
  height: 20px;
  flex-shrink: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}

.ip-toast__icon svg {
  width: 20px;
  height: 20px;
}

/* Body — 合并时内容 crossfade（§6.6） */
.ip-toast__body {
  flex: 1;
  min-width: 0;
}
.ip-toast--merging .ip-toast__body {
  animation: ip-toast-content-fade var(--ip-duration-base) var(--ip-ease-out);
}

.ip-toast__title {
  font-size: var(--ip-text-body-sm-size);
  font-weight: var(--ip-font-weight-medium);
  color: var(--ip-color-text-primary);
  margin-bottom: 2px;
}

.ip-toast__message {
  color: var(--ip-color-text-body);
  word-break: break-word;
}

/* Close button */
.ip-toast__close {
  width: 20px;
  height: 20px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  background: transparent;
  border: none;
  border-radius: var(--ip-radius-sm);
  color: var(--ip-color-text-tertiary);
  cursor: pointer;
  transition:
    background-color var(--ip-duration-fast) var(--ip-ease-out),
    color           var(--ip-duration-fast) var(--ip-ease-out),
    transform       var(--ip-duration-fast) var(--ip-ease-out);
}

.ip-toast__close:hover {
  background: var(--ip-color-bg-tertiary);
  color: var(--ip-color-icon-default);
}

.ip-toast__close:active {
  transform: scale(0.92);
}

.ip-toast__close:focus-visible {
  outline: none;
  box-shadow: var(--ip-shadow-focus);
}

.ip-toast__close svg {
  width: 14px;
  height: 14px;
}
</style>
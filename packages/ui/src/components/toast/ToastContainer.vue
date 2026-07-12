<script setup lang="ts">
/**
 * ToastContainer — Toast 容器（全局）
 *
 * 用法：
 *   1. 在 app 入口处 provide Toast API：
 *      const toast = provideToast()
 *   2. 在 App.vue 顶层挂载一次：
 *      <ToastContainer />
 *   3. 任意位置调用：
 *      const toast = useToast()
 *      toast.success('已保存')
 *
 * 规范：§2.6.1 — 位置 top-right（默认），z-index 1400
 * 微交互（§6）：
 *  - enter: translateX + opacity 200ms ease-out
 *  - exit:  内容 fade 150ms + 高度 collapse 200ms (50ms delay)
 *  - 兄弟移动: 200ms ease-out（TransitionGroup move class）
 */
import { useToast } from './useToast'
import type { ToastPosition } from './types'
import Toast from './Toast.vue'

interface Props {
  /** 容器位置 */
  position?: ToastPosition
}

const props = withDefaults(defineProps<Props>(), {
  position: 'top-right',
})

const toastApi = useToast()

function onClose(id: string): void {
  toastApi.remove(id)
}

function onPause(id: string): void {
  toastApi.pause(id)
}

function onResume(id: string): void {
  toastApi.resume(id)
}

const positionStyle = (): Record<string, string> => {
  const offset = `var(--ip-toast-offset)`
  const map: Record<ToastPosition, Record<string, string>> = {
    'top-right':    { top: offset, right: offset },
    'top-left':     { top: offset, left: offset },
    'top-center':   { top: offset, left: '50%', transform: 'translateX(-50%)' },
    'bottom-right': { bottom: offset, right: offset },
    'bottom-left':  { bottom: offset, left: offset },
    'bottom-center': { bottom: offset, left: '50%', transform: 'translateX(-50%)' },
  }
  return map[props.position] ?? map['top-right']
}
</script>

<template>
  <Teleport to="body">
    <div
      :class="['ip-toast-container', `ip-toast-container--${props.position}`]"
      :style="positionStyle()"
      aria-live="polite"
      aria-atomic="true"
    >
      <TransitionGroup name="ip-toast" tag="div" class="ip-toast-container__list">
        <Toast
          v-for="t in toastApi.toasts.value"
          :key="t.id"
          :toast="t"
          @close="onClose(t.id)"
          @pause="onPause(t.id)"
          @resume="onResume(t.id)"
        />
      </TransitionGroup>
    </div>
  </Teleport>
</template>

<style scoped>
/* ============================================================
 * Toast Container（§6.2）
 * ============================================================ */
.ip-toast-container {
  position: fixed;
  display: flex;
  flex-direction: column;
  gap: var(--ip-toast-gap);
  z-index: var(--ip-z-toast);
  pointer-events: none;        /* 容器不拦截 */
  max-width: calc(100vw - 32px);
}

.ip-toast-container__list {
  display: flex;
  flex-direction: column;
  gap: var(--ip-toast-gap);
  position: relative;
}

/* ===========================
 * Transition（§6.3 / §6.4）
 * =========================== */

/* ----- Enter：translateX + opacity 200ms ease-out ----- */
.ip-toast-enter-from {
  opacity: 0;
  transform: translateX(20px);
}
.ip-toast-enter-active {
  transition:
    opacity   var(--ip-duration-toast-in) var(--ip-ease-out),
    transform var(--ip-duration-toast-in) var(--ip-ease-out);
}
.ip-toast-enter-to {
  opacity: 1;
  transform: translateX(0);
}

/* ----- Leave：内容 fade 150ms + 高度 collapse 200ms (50ms delay) ----- */
.ip-toast-leave-from {
  opacity: 1;
  transform: translateX(0);
  max-height: 200px;
  margin-bottom: var(--ip-toast-gap);
}
.ip-toast-leave-active {
  transition:
    opacity       var(--ip-duration-toast-out) var(--ip-ease-in),
    transform     var(--ip-duration-toast-out) var(--ip-ease-in),
    max-height    var(--ip-duration-toast-collapse) var(--ip-ease-in) var(--ip-duration-immediate),
    margin-bottom var(--ip-duration-toast-collapse) var(--ip-ease-in) var(--ip-duration-immediate),
    padding-top   var(--ip-duration-toast-collapse) var(--ip-ease-in) var(--ip-duration-immediate),
    padding-bottom var(--ip-duration-toast-collapse) var(--ip-ease-in) var(--ip-duration-immediate);
  /* 退出时绝对定位，不占据 flex 流，其他 toast 可以平滑上移 */
  position: absolute;
  right: 0;
  left: 0;
}
.ip-toast-leave-to {
  opacity: 0;
  transform: translateX(20px);
  max-height: 0;
  margin-bottom: 0;
  padding-top: 0;
  padding-bottom: 0;
}

/* ----- Move：兄弟位置变化 200ms ease-out（§6.5） ----- */
.ip-toast-move {
  transition: transform var(--ip-duration-message) var(--ip-ease-out);
}
</style>
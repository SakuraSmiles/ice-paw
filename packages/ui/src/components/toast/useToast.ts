/**
 * useToast — 全局 Toast composable
 *
 * 规范：icepaw-design-system.md §2.6
 * - 位置：top-right（默认）
 * - 最多同时显示 3 个
 * - 同类型合并策略（v1.0.1）：
 *   - 内容替换（message / title）
 *   - 重置 duration
 *   - 保留原 ID（避免动画闪烁）
 *   - 重启计时器
 * - 悬停暂停（v1.0 §6.7）：记录 remaining 与 pausedAt
 */

import { inject, provide, ref, type Ref } from 'vue'
import { ToastApiKey, type ToastApi, type ToastInstance, type ToastOptions, type ToastType } from './types'

const MAX_VISIBLE = 3

/* 默认 duration 按类型 */
function defaultDuration(type: ToastType): number {
  switch (type) {
    case 'success': return 3000
    case 'info':    return 3000
    case 'warning': return 5000
    case 'error':   return 8000
    default:        return 3000
  }
}

/* 简易 nanoid */
function nanoid(): string {
  return `t-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`
}

/**
 * 创建一个新的 Toast API（用于 app 顶层 provide）。
 * 通常在 app 入口 createApp().provide(...) 一次即可。
 */
export function createToastApi(): ToastApi {
  const toasts: Ref<ToastInstance[]> = ref<ToastInstance[]>([])
  const timers = new Map<string, ReturnType<typeof setTimeout>>()

  function startTimer(t: ToastInstance, duration: number): void {
    if (timers.has(t.id)) {
      clearTimeout(timers.get(t.id)!)
      timers.delete(t.id)
    }
    if (duration > 0) {
      const handle = setTimeout(() => {
        remove(t.id)
      }, duration)
      timers.set(t.id, handle)
    }
  }

  function resetTimer(t: ToastInstance, duration?: number): void {
    const dur = duration ?? t.duration
    t.duration = dur
    t.remaining = dur
    startTimer(t, dur)
  }

  function push(opts: ToastOptions): ToastInstance {
    const merged: ToastOptions = {
      closable: true,
      mergeable: true,
      duration: opts.duration ?? defaultDuration(opts.type),
      ...opts,
    }

    /* 同类型合并（v1.0.1） */
    if (merged.mergeable !== false) {
      const existing = toasts.value.find((t) => t.type === opts.type)
      if (existing) {
        existing.message = merged.message ?? ''
        existing.title = merged.title ?? ''
        existing.duration = merged.duration ?? defaultDuration(opts.type)
        existing.remaining = existing.duration
        existing.createdAt = Date.now()
        existing.isMerging = true
        existing.pausedAt = null
        // 150ms 后清除 merging 标记（§6.6）
        setTimeout(() => {
          existing.isMerging = false
        }, 150)
        resetTimer(existing)
        return existing
      }
    }

    /* 添加新 toast，超过 3 个时挤掉最早的 */
    if (toasts.value.length >= MAX_VISIBLE) {
      const removed = toasts.value.shift()
      if (removed) remove(removed.id)
    }

    const instance: ToastInstance = {
      id: nanoid(),
      type: merged.type,
      title: merged.title ?? '',
      message: merged.message ?? '',
      duration: merged.duration ?? defaultDuration(merged.type),
      remaining: merged.duration ?? defaultDuration(merged.type),
      closable: merged.closable ?? true,
      mergeable: merged.mergeable ?? true,
      isMerging: false,
      pausedAt: null,
      createdAt: Date.now(),
    }
    toasts.value.push(instance)
    resetTimer(instance)
    return instance
  }

  function remove(id: string): void {
    const idx = toasts.value.findIndex((t) => t.id === id)
    if (idx >= 0) {
      toasts.value.splice(idx, 1)
      const handle = timers.get(id)
      if (handle) {
        clearTimeout(handle)
        timers.delete(id)
      }
    }
  }

  function clear(): void {
    toasts.value.splice(0)
    timers.forEach((h) => clearTimeout(h))
    timers.clear()
  }

  /* 悬停暂停 / 恢复（§6.7） */
  function pause(id: string): void {
    const t = toasts.value.find((x) => x.id === id)
    if (!t || t.pausedAt) return
    const handle = timers.get(id)
    if (handle) {
      clearTimeout(handle)
      timers.delete(id)
      t.pausedAt = Date.now()
    }
  }

  function resume(id: string): void {
    const t = toasts.value.find((x) => x.id === id)
    if (!t || !t.pausedAt) return
    const elapsed = Date.now() - t.pausedAt
    const remaining = Math.max(0, (t.remaining ?? t.duration) - elapsed)
    t.pausedAt = null
    t.remaining = remaining
    startTimer(t, remaining)
  }

  /* 快捷方法 */
  function make(type: ToastType) {
    return (message: string, opts?: Partial<ToastOptions>): ToastInstance =>
      push({ type, message, ...(opts ?? {}) })
  }

  return {
    toasts,
    push,
    remove,
    clear,
    pause,
    resume,
    success: make('success'),
    error:   make('error'),
    warning: make('warning'),
    info:    make('info'),
  }
}

/**
 * 在组件中取 Toast API。
 * 优先取 inject（由 provider 共享），否则创建一个本地实例（独立、单组件作用域）。
 */
export function useToast(): ToastApi {
  const existing = inject(ToastApiKey, null)
  if (existing) return existing
  return createToastApi()
}

/**
 * 在 app 顶层注册 Toast API（provide）。
 * 用法（在 main.ts / App.vue 里）：
 *   const toast = provideToast()
 *   // 不需要组件树共享时，可直接 push
 */
export function provideToast(): ToastApi {
  const api = createToastApi()
  provide(ToastApiKey, api)
  return api
}
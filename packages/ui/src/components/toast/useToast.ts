import { inject, provide, ref, type Ref } from 'vue'
import { ToastApiKey, type ToastApi, type ToastInstance, type ToastOptions, type ToastType } from './types'

/** REQ-UI-006C：最大同时可见 toast 数（v4: 3 → 5） */
const MAX_VISIBLE = 5

/* 默认 duration 按类型；REQ-UI-006B: loading 用 Infinity（永不自动关闭） */
function defaultDuration(type: ToastType): number {
  switch (type) {
    case 'success': return 3000
    case 'info':    return 3000
    case 'warning': return 5000
    case 'error':   return 8000
    case 'loading': return Number.POSITIVE_INFINITY
    default:        return 3000
  }
}

/* 简易 nanoid */
function nanoid(): string {
  return `t-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`
}

export function createToastApi(): ToastApi {
  const toasts: Ref<ToastInstance[]> = ref<ToastInstance[]>([])
  const timers = new Map<string, ReturnType<typeof setTimeout>>()

  function startTimer(t: ToastInstance, duration: number): void {
    if (timers.has(t.id)) {
      clearTimeout(timers.get(t.id)!)
      timers.delete(t.id)
    }
    /*
     * REQ-UI-006B：loading 类型用 Infinity 表示永不自动关闭。
     * Infinity 不是有限正数，必须跳过定时器（否则 setTimeout(fn, Infinity) 立即触发）。
     */
    if (Number.isFinite(duration) && duration > 0) {
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
        setTimeout(() => {
          existing.isMerging = false
        }, 150)
        resetTimer(existing)
        return existing
      }
    }

    /* REQ-UI-006C：超过 5 个时挤掉最早的 */
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

  /**
   * REQ-UI-006B：loading 类型快捷方法。
   * 默认 closable=false；不自动消失。
   * 返回 toast 实例，可用于后续 toast.update(id, ...) 更新为成功/失败。
   */
  function loading(message: string, opts?: Partial<ToastOptions>): ToastInstance {
    return push({
      type: 'loading',
      message,
      closable: false,
      ...opts,
    })
  }

  /**
   * REQ-UI-006B：按 id 原地更新已有 toast 的部分字段。
   * 典型场景：loading → success（请求完成）/ loading → error（请求失败）。
   * 若 id 不存在，返回 undefined。
   */
  function update(
    id: string,
    patch: Partial<Pick<ToastOptions, 'type' | 'title' | 'message' | 'duration' | 'closable' | 'mergeable'>>,
  ): ToastInstance | undefined {
    const t = toasts.value.find((x) => x.id === id)
    if (!t) return undefined
    if (patch.type !== undefined) t.type = patch.type
    if (patch.title !== undefined) t.title = patch.title
    if (patch.message !== undefined) t.message = patch.message
    if (patch.closable !== undefined) t.closable = patch.closable
    if (patch.mergeable !== undefined) t.mergeable = patch.mergeable
    if (patch.duration !== undefined) t.duration = patch.duration
    /* 切换 type 涉及到 duration 变更时重置定时器 */
    const newDur = patch.duration ?? defaultDuration(t.type)
    if (patch.type !== undefined || patch.duration !== undefined) {
      t.duration = newDur
      t.remaining = newDur
      t.createdAt = Date.now()
      t.isMerging = true
      t.pausedAt = null
      setTimeout(() => {
        t.isMerging = false
      }, 150)
      resetTimer(t, newDur)
    }
    return t
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
    loading,
    update,
  }
}

export function useToast(): ToastApi {
  const existing = inject(ToastApiKey, null)
  if (existing) return existing
  return createToastApi()
}

export function provideToast(): ToastApi {
  const api = createToastApi()
  provide(ToastApiKey, api)
  return api
}

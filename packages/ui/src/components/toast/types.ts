/**
 * Toast — 公开 Props / Emits + 全局类型
 *
 * 规范：icepaw-design-system.md §2.6
 * 类型：success / error / warning / info
 * 位置：top-right（默认）
 * 时长：success=3000 / info=3000 / warning=5000 / error=8000
 * 合并策略（v1.0.1）：同 type 替换内容 + 重置 duration
 * 悬停暂停（v1.0 §6.7）
 */

import type { InjectionKey, Ref } from 'vue'

export type ToastType = 'success' | 'error' | 'warning' | 'info'
export type ToastPosition = 'top-right' | 'top-left' | 'bottom-right' | 'bottom-left' | 'top-center' | 'bottom-center'

export interface ToastOptions {
  /** 类型（必填） */
  type: ToastType
  /** 标题 */
  title?: string
  /** 内容（详细描述） */
  message?: string
  /** 默认 slot 内容（如果 message 不够，可以放自定义 HTML/组件） */
  /** 自定义时长（ms），不传则按 type 默认值 */
  duration?: number
  /** 是否可手动关闭（默认 true） */
  closable?: boolean
  /** 同类型是否合并（默认 true） */
  mergeable?: boolean
}

export interface ToastInstance extends Required<Pick<ToastOptions, 'type' | 'message' | 'title' | 'closable' | 'mergeable'>> {
  id: string
  duration: number
  /** 当前剩余时长（ms）— 用于悬停暂停 */
  remaining?: number
  /** 是否处于合并动画中（§6.6） */
  isMerging?: boolean
  /** 暂停起始时间戳（ms），null 表示未暂停 */
  pausedAt?: number | null
  createdAt: number
}

/* useToast 注入的全局 toasts ref + API */
export interface ToastApi {
  toasts: Ref<ToastInstance[]>
  push(opts: ToastOptions): ToastInstance
  remove(id: string): void
  clear(): void
  pause(id: string): void
  resume(id: string): void
  success(message: string, opts?: Partial<ToastOptions>): ToastInstance
  error(message: string, opts?: Partial<ToastOptions>): ToastInstance
  warning(message: string, opts?: Partial<ToastOptions>): ToastInstance
  info(message: string, opts?: Partial<ToastOptions>): ToastInstance
}

/* provide / inject key */
export const ToastApiKey: InjectionKey<ToastApi> = Symbol('IcePawToast')
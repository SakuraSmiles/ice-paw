/**
 * Toast — 公开 Props / Emits + 全局类型
 *
 * 规范：icepaw-design-system.md §2.6
 * 类型：success / error / warning / info / loading（5 种，v4 REQ-UI-006B）
 * 位置：top-right（默认）
 * 时长：success=3000 / info=3000 / warning=5000 / error=8000 / loading=Infinity
 * 队列上限：5（v4 REQ-UI-006C）
 * 合并策略（v1.0.1）：同 type 替换内容 + 重置 duration
 * 悬停暂停（v1.0 §6.7）
 */

import type { InjectionKey, Ref } from 'vue'

/**
 * REQ-UI-006B：增加 `loading` 类型。
 * loading 类型展示转圈图标、不自动消失（duration=Infinity），
 * 由调用方手动调用 toast.update(id, {type:'success'}) 或 toast.remove(id) 更新或关闭。
 */
export type ToastType = 'success' | 'error' | 'warning' | 'info' | 'loading'
export type ToastPosition = 'top-right' | 'top-left' | 'bottom-right' | 'bottom-left' | 'top-center' | 'bottom-center'

export interface ToastOptions {
  /** 类型（必填） */
  type: ToastType
  /** 标题 */
  title?: string
  /** 内容（详细描述） */
  message?: string
  /** 默认 slot 内容（如果 message 不够，可以放自定义 HTML/组件） */
  /** 自定义时长（ms），不传则按 type 默认值。loading 类型忽略此项（始终不自动关闭） */
  duration?: number
  /** 是否可手动关闭（默认 true；loading 类型通常 false） */
  closable?: boolean
  /** 同类型是否合并（默认 true） */
  mergeable?: boolean
}

export interface ToastInstance extends Required<Pick<ToastOptions, 'type' | 'message' | 'title' | 'closable' | 'mergeable'>> {
  id: string
  duration: number
  /** 当前剩余时长（ms）— 用于悬停暂停。loading 类型固定 Infinity */
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
  /**
   * REQ-UI-006B：loading 类型快捷方法。
   * 默认 closable=false；不自动消失。
   * 返回 toast 实例（持有 id），可用于后续 toast.update(id, ...) 更新为成功/失败。
   */
  loading(message: string, opts?: Partial<ToastOptions>): ToastInstance
  /** REQ-UI-006B：按 id 原地更新已有 toast 的部分字段，用于 loading→success/error 流转 */
  update(id: string, patch: Partial<Pick<ToastOptions, 'type' | 'title' | 'message' | 'duration' | 'closable' | 'mergeable'>>): ToastInstance | undefined
}

/* provide / inject key */
export const ToastApiKey: InjectionKey<ToastApi> = Symbol('IcePawToast')
